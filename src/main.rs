use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use reqwest::{Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "ssp",
    version,
    about = "Statespace experimentation and A/B testing"
)]
struct Cli {
    #[arg(long, env = "STATESPACE_URL", global = true)]
    endpoint: Option<String>,
    #[arg(
        long = "account-token",
        env = "STATESPACE_ACCOUNT_TOKEN",
        global = true,
        hide_env_values = true
    )]
    token: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in with GitHub or Google in your browser.
    Login {
        /// Print the login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Remove the saved account session.
    Logout,
    /// Manage projects.
    Project(ProjectCommand),
    /// Show the signed-in account and enforced plan limits.
    Account,
    #[command(hide = true)]
    /// Run internal service administration commands.
    Admin(AdminCommand),
    /// Manage experiments in a project.
    Experiment(ExperimentCommand),
}

#[derive(Args)]
struct AdminCommand {
    #[command(subcommand)]
    command: AdminSubcommand,
}

#[derive(Args)]
struct ProjectCommand {
    #[command(subcommand)]
    command: ProjectSubcommand,
}

#[derive(Subcommand)]
enum ProjectSubcommand {
    /// Create a globally named project.
    Create {
        #[arg(short = 'N', long)]
        name: String,
    },
    /// List all projects.
    List,
    /// Show one project, its tokens, and its experiments.
    Show {
        #[arg(short = 'N', long)]
        name: String,
    },
    /// Manage project access tokens.
    Token(ProjectTokenCommand),
}

#[derive(Args)]
struct ProjectTokenCommand {
    #[command(subcommand)]
    command: ProjectTokenSubcommand,
}

#[derive(Subcommand)]
enum ProjectTokenSubcommand {
    /// Create a project token and print its secret once.
    Create {
        #[arg(short = 'P', long)]
        project: String,
        #[arg(short = 'N', long)]
        name: String,
        #[arg(long)]
        access: TokenAccess,
    },
    /// List active project tokens without their secrets.
    List {
        #[arg(short = 'P', long)]
        project: String,
    },
    /// Revoke a project token.
    Revoke {
        #[arg(short = 'P', long)]
        project: String,
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum AdminSubcommand {
    /// Change an account plan.
    SetPlan {
        #[arg(long)]
        account: String,
        #[arg(long, value_parser = ["free", "pro", "enterprise"])]
        plan: String,
    },
}

#[derive(Args)]
struct ExperimentCommand {
    #[command(subcommand)]
    command: ExperimentSubcommand,
}

#[derive(Subcommand)]
enum ExperimentSubcommand {
    /// Create an experiment from YAML.
    Create {
        #[arg(short, long)]
        file: PathBuf,
        #[arg(short = 'P', long)]
        project: String,
    },
    /// Replace a draft or stopped experiment from YAML.
    Update {
        #[arg(short, long)]
        file: PathBuf,
        #[arg(short = 'P', long)]
        project: String,
    },
    /// List experiments.
    List {
        #[arg(short = 'P', long)]
        project: String,
    },
    /// Show one experiment.
    Show {
        #[arg(short = 'N', long)]
        name: String,
        #[arg(short = 'P', long)]
        project: String,
    },
    /// Start an experiment.
    Start {
        #[arg(short = 'N', long)]
        name: String,
        #[arg(short = 'P', long)]
        project: String,
    },
    /// Stop an experiment.
    Stop {
        #[arg(short = 'N', long)]
        name: String,
        #[arg(short = 'P', long)]
        project: String,
    },
    /// Delete an experiment and its recorded data.
    Delete {
        #[arg(short = 'N', long)]
        name: String,
        #[arg(short = 'P', long)]
        project: String,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Settings {
    endpoint: Option<String>,
    token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LoginStart {
    device_code: String,
    verification_url: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct LoginComplete {
    token: String,
    account: LoginAccount,
}

#[derive(Debug, Deserialize, Serialize)]
struct LoginAccount {
    id: String,
    provider: String,
    login: String,
    plan: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LoginOutput {
    status: &'static str,
    account: LoginAccount,
}

#[derive(Serialize)]
struct ErrorOutput {
    error: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectResponse {
    id: String,
    url: String,
    token: ProjectTokenSecret,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectDetails {
    id: String,
    url: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectView {
    id: String,
    url: String,
    tokens: Vec<ProjectToken>,
    experiments: Vec<ExperimentView>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum TokenAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectToken {
    id: String,
    name: String,
    access: TokenAccess,
    prefix: String,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectTokenSecret {
    #[serde(flatten)]
    details: ProjectToken,
    token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct AccountDetails {
    id: String,
    provider: String,
    login: String,
    plan: String,
    usage_bytes: u64,
    storage_limit_bytes: Option<u64>,
    write_events_per_minute: Option<u64>,
    retention_days: Option<u64>,
    project_limit: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExperimentDefinition {
    name: String,
    description: String,
    #[serde(default = "default_assignment")]
    assignment: String,
    groups: Vec<ExperimentGroup>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExperimentGroup {
    name: String,
    weight: f64,
    #[serde(default)]
    config: Value,
}

#[derive(Debug, Deserialize, Serialize)]
struct ExperimentView {
    name: String,
    description: String,
    assignment: String,
    status: String,
    groups: Vec<ExperimentGroup>,
}

fn default_assignment() -> String {
    "subject_id".into()
}

struct Api {
    client: Client,
    endpoint: String,
    token: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        let output = serde_yaml_ng::to_string(&ErrorOutput {
            error: format!("{error:#}"),
        })
        .unwrap_or_else(|_| "error: unknown error\n".into());
        eprint!("{output}");
        if !output.ends_with('\n') {
            eprintln!();
        }
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut settings = Settings::load()?;
    let endpoint = cli
        .endpoint
        .clone()
        .or_else(|| settings.endpoint.clone())
        .unwrap_or_else(|| "https://api.statespace.com".into())
        .trim_end_matches('/')
        .to_owned();
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("statespace-cli/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let api = Api {
        client,
        endpoint: endpoint.clone(),
        token: cli.token.clone().or_else(|| settings.token.clone()),
    };

    match cli.command {
        Command::Login { no_open } => {
            let login = start_login(&api).await?;
            let printed_url = no_open || webbrowser::open(&login.verification_url).is_err();
            if printed_url {
                print_yaml(&json!({ "login_url": login.verification_url }))?;
                eprintln!("Open the login URL to sign in or create a free account.");
            } else {
                eprintln!("Complete sign-in in your browser.");
            }
            let complete = wait_for_login(&api, &login).await?;
            settings = Settings {
                endpoint: Some(endpoint),
                token: Some(complete.token),
            };
            settings.save()?;
            if printed_url {
                println!("---");
            }
            print_yaml(&LoginOutput {
                status: "authenticated",
                account: complete.account,
            })?;
        }
        Command::Logout => {
            settings.endpoint = Some(endpoint);
            settings.token = None;
            settings.save()?;
            print_yaml(&json!({ "status": "logged-out" }))?;
        }
        Command::Project(command) => match command.command {
            ProjectSubcommand::Create { name } => {
                let response = api
                    .request(Method::POST, "/v1/projects")?
                    .json(&json!({ "name": name }))
                    .send()
                    .await?;
                print_yaml(&decode::<ProjectResponse>(response).await?)?;
            }
            ProjectSubcommand::List => {
                let response = api.request(Method::GET, "/v1/projects")?.send().await?;
                print_yaml(&decode::<Vec<ProjectDetails>>(response).await?)?;
            }
            ProjectSubcommand::Show { name } => {
                let response = api
                    .request(Method::GET, &format!("/v1/projects/{name}"))?
                    .send()
                    .await?;
                print_yaml(&decode::<ProjectView>(response).await?)?;
            }
            ProjectSubcommand::Token(command) => run_project_token(&api, command).await?,
        },
        Command::Account => {
            let response = api.request(Method::GET, "/v1/account")?.send().await?;
            print_yaml(&decode::<AccountDetails>(response).await?)?;
        }
        Command::Admin(command) => {
            let admin_api = Api {
                client: api.client.clone(),
                endpoint: api.endpoint.clone(),
                token: Some(
                    std::env::var("STATESPACE_ADMIN_TOKEN")
                        .context("STATESPACE_ADMIN_TOKEN is required")?,
                ),
            };
            match command.command {
                AdminSubcommand::SetPlan { account, plan } => {
                    let response = admin_api
                        .request(Method::PATCH, &format!("/v1/admin/accounts/{account}"))?
                        .json(&json!({ "plan": plan }))
                        .send()
                        .await?;
                    print_yaml(&decode::<AccountDetails>(response).await?)?;
                }
            }
        }
        Command::Experiment(command) => run_experiment(&api, command).await?,
    }
    Ok(())
}

async fn run_project_token(api: &Api, command: ProjectTokenCommand) -> anyhow::Result<()> {
    match command.command {
        ProjectTokenSubcommand::Create {
            project,
            name,
            access,
        } => {
            let response = api
                .request(Method::POST, &format!("/v1/projects/{project}/tokens"))?
                .json(&json!({ "name": name, "access": access }))
                .send()
                .await?;
            print_yaml(&decode::<ProjectTokenSecret>(response).await?)?;
        }
        ProjectTokenSubcommand::List { project } => {
            let response = api
                .request(Method::GET, &format!("/v1/projects/{project}/tokens"))?
                .send()
                .await?;
            print_yaml(&decode::<Vec<ProjectToken>>(response).await?)?;
        }
        ProjectTokenSubcommand::Revoke { project, id } => {
            let response = api
                .request(
                    Method::DELETE,
                    &format!("/v1/projects/{project}/tokens/{id}"),
                )?
                .send()
                .await?;
            ensure_success(response).await?;
            print_yaml(&json!({ "revoked": id }))?;
        }
    }
    Ok(())
}

async fn start_login(api: &Api) -> anyhow::Result<LoginStart> {
    let response = api
        .client
        .post(format!("{}/v1/auth/device", api.endpoint))
        .send()
        .await?;
    decode(response).await
}

async fn wait_for_login(api: &Api, login: &LoginStart) -> anyhow::Result<LoginComplete> {
    let deadline = Instant::now() + Duration::from_secs(login.expires_in);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(login.interval.max(1))).await;
        let response = api
            .client
            .post(format!("{}/v1/auth/device/token", api.endpoint))
            .json(&json!({ "device_code": login.device_code }))
            .send()
            .await?;
        if response.status() == StatusCode::ACCEPTED {
            continue;
        }
        return decode(response).await;
    }
    bail!("login expired; run ssp login again")
}

async fn run_experiment(api: &Api, command: ExperimentCommand) -> anyhow::Result<()> {
    match command.command {
        ExperimentSubcommand::Create { file, project } => {
            let definition = read_experiment_definition(&file)?;
            let response = api
                .project_request(Method::POST, "/v1/experiments", &project)?
                .json(&definition)
                .send()
                .await?;
            print_yaml(&decode::<ExperimentView>(response).await?)?;
        }
        ExperimentSubcommand::Update { file, project } => {
            let definition = read_experiment_definition(&file)?;
            let response = api
                .project_request(
                    Method::PUT,
                    &format!("/v1/experiments/{}", definition.name),
                    &project,
                )?
                .json(&definition)
                .send()
                .await?;
            print_yaml(&decode::<ExperimentView>(response).await?)?;
        }
        ExperimentSubcommand::List { project } => {
            let response = api
                .project_request(Method::GET, "/v1/experiments", &project)?
                .send()
                .await?;
            print_yaml(&decode::<Vec<ExperimentView>>(response).await?)?;
        }
        ExperimentSubcommand::Show { name, project } => {
            let response = api
                .project_request(Method::GET, &format!("/v1/experiments/{name}"), &project)?
                .send()
                .await?;
            print_yaml(&decode::<ExperimentView>(response).await?)?;
        }
        ExperimentSubcommand::Start { name, project } => {
            set_experiment_status(api, &project, &name, "running").await?;
        }
        ExperimentSubcommand::Stop { name, project } => {
            set_experiment_status(api, &project, &name, "stopped").await?;
        }
        ExperimentSubcommand::Delete { name, project } => {
            let response = api
                .project_request(Method::DELETE, &format!("/v1/experiments/{name}"), &project)?
                .send()
                .await?;
            ensure_success(response).await?;
            print_yaml(&json!({ "deleted": true, "experiment": name }))?;
        }
    }
    Ok(())
}

impl Settings {
    fn path() -> anyhow::Result<PathBuf> {
        if let Some(path) = std::env::var_os("STATESPACE_CONFIG") {
            return Ok(PathBuf::from(path));
        }
        Ok(dirs::config_dir()
            .context("configuration directory is unavailable")?
            .join("statespace/config.toml"))
    }

    fn load() -> anyhow::Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
    }

    fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().context("invalid configuration path")?)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, toml::to_string_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

impl Api {
    fn request(&self, method: Method, path: &str) -> anyhow::Result<reqwest::RequestBuilder> {
        let token = self
            .token
            .as_deref()
            .context("not logged in; run ssp login")?;
        Ok(self
            .client
            .request(method, format!("{}{}", self.endpoint, path))
            .bearer_auth(token))
    }

    fn project_request(
        &self,
        method: Method,
        path: &str,
        project: &str,
    ) -> anyhow::Result<reqwest::RequestBuilder> {
        Ok(self
            .request(method, path)?
            .header("X-Statespace-Project", project))
    }
}

async fn decode<T: serde::de::DeserializeOwned>(response: Response) -> anyhow::Result<T> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        if status == StatusCode::UNAUTHORIZED {
            bail!("session expired or invalid; run ssp login");
        }
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| value.get("error")?.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
        bail!("server returned {status}: {message}");
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn ensure_success(response: Response) -> anyhow::Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let bytes = response.bytes().await?;
    if status == StatusCode::UNAUTHORIZED {
        bail!("session expired or invalid; run ssp login");
    }
    let message = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
    bail!("server returned {status}: {message}")
}

async fn set_experiment_status(
    api: &Api,
    project: &str,
    name: &str,
    status: &str,
) -> anyhow::Result<()> {
    let response = api
        .project_request(
            Method::POST,
            &format!("/v1/experiments/{name}/state"),
            project,
        )?
        .json(&json!({ "status": status }))
        .send()
        .await?;
    print_yaml(&decode::<ExperimentView>(response).await?)
}

fn read_experiment_definition(path: &Path) -> anyhow::Result<ExperimentDefinition> {
    if !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yaml" | "yml")
    ) {
        bail!("experiment definition must use a .yaml or .yml file");
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read experiment file: {}", path.display()))?;
    parse_experiment_definition(&contents)
        .with_context(|| format!("invalid experiment file: {}", path.display()))
}

fn parse_experiment_definition(contents: &str) -> anyhow::Result<ExperimentDefinition> {
    let definition: ExperimentDefinition = serde_yaml_ng::from_str(contents)?;
    if definition.name.trim().is_empty() {
        bail!("experiment name must not be empty");
    }
    if definition.description.trim().is_empty() {
        bail!("experiment description must not be empty");
    }
    if definition.assignment.trim().is_empty() {
        bail!("experiment assignment must not be empty");
    }
    if definition.groups.is_empty() {
        bail!("experiment file must define at least one treatment group");
    }
    let mut group_names = HashSet::with_capacity(definition.groups.len());
    for group in &definition.groups {
        if group.name.trim().is_empty() {
            bail!("group name must not be empty");
        }
        if !group_names.insert(group.name.as_str()) {
            bail!("group names must be unique");
        }
        if !group.weight.is_finite() || group.weight <= 0.0 {
            bail!("group weight must be a finite number greater than zero");
        }
    }
    let has_control = definition
        .groups
        .iter()
        .any(|group| group.name == "control");
    if has_control && definition.groups.len() < 2 {
        bail!("an explicit control group requires at least one treatment group");
    }
    if !has_control {
        let treatment_weight: f64 = definition.groups.iter().map(|group| group.weight).sum();
        if treatment_weight >= 1.0 {
            bail!("treatment weights must leave a positive weight for the default control group");
        }
    } else {
        let total_weight: f64 = definition.groups.iter().map(|group| group.weight).sum();
        if (total_weight - 1.0).abs() > 0.000000001 {
            bail!("group weights must sum to one when control is explicit");
        }
    }
    Ok(definition)
}

fn print_yaml<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let output = serde_yaml_ng::to_string(value)?;
    print!("{output}");
    if !output.ends_with('\n') {
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_commands_and_short_options() {
        assert!(matches!(
            Cli::try_parse_from(["ssp", "login"]).unwrap().command,
            Command::Login { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["ssp", "logout"]).unwrap().command,
            Command::Logout
        ));
        assert!(matches!(
            Cli::try_parse_from(["ssp", "project", "create", "-N", "support-agents"])
                .unwrap()
                .command,
            Command::Project(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ssp", "project", "list"])
                .unwrap()
                .command,
            Command::Project(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ssp", "project", "show", "--name", "support-agents"])
                .unwrap()
                .command,
            Command::Project(_)
        ));
        assert!(Cli::try_parse_from(["ssp", "create", "--name", "support-agents"]).is_err());
        Cli::try_parse_from([
            "ssp",
            "project",
            "token",
            "create",
            "-P",
            "support-agents",
            "-N",
            "production",
            "--access",
            "read-write",
        ])
        .unwrap();
        Cli::try_parse_from([
            "ssp",
            "project",
            "token",
            "revoke",
            "--project",
            "support-agents",
            "--id",
            "tok_123",
        ])
        .unwrap();
    }

    #[test]
    fn requires_a_project_for_experiment_commands() {
        Cli::try_parse_from(["ssp", "experiment", "list", "-P", "support-agents"]).unwrap();
        assert!(Cli::try_parse_from(["ssp", "experiment", "list"]).is_err());
    }

    #[test]
    fn parses_account_and_plan_admin_commands() {
        assert!(matches!(
            Cli::try_parse_from(["ssp", "account"]).unwrap().command,
            Command::Account
        ));
        Cli::try_parse_from([
            "ssp",
            "admin",
            "set-plan",
            "--account",
            "acct_123",
            "--plan",
            "pro",
        ])
        .unwrap();
        Cli::try_parse_from([
            "ssp",
            "admin",
            "set-plan",
            "--account",
            "acct_123",
            "--plan",
            "enterprise",
        ])
        .unwrap();
        assert!(
            Cli::try_parse_from([
                "ssp",
                "admin",
                "set-plan",
                "--account",
                "acct_123",
                "--plan",
                "starter",
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_yaml_experiment() {
        let definition = parse_experiment_definition(
            "name: ranking-v2\ndescription: Compare two ranking models.\ngroups:\n  - name: treatment\n    weight: 0.5\n    config:\n      model: gpt-5\n",
        )
        .unwrap();
        assert_eq!(definition.assignment, "subject_id");
        assert_eq!(definition.groups[0].config["model"], "gpt-5");
    }

    #[test]
    fn requires_an_experiment_description() {
        let result = parse_experiment_definition(
            "name: ranking-v2\ngroups:\n  - name: treatment\n    weight: 0.5\n",
        );
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_assignment_and_groups() {
        let empty_assignment = parse_experiment_definition(
            "name: ranking-v2\ndescription: Test ranking.\nassignment: ''\ngroups:\n  - name: treatment\n    weight: 0.5\n",
        );
        assert!(empty_assignment.is_err());

        let duplicate_groups = parse_experiment_definition(
            "name: ranking-v2\ndescription: Test ranking.\ngroups:\n  - name: treatment\n    weight: 0.2\n  - name: treatment\n    weight: 0.2\n",
        );
        assert!(duplicate_groups.is_err());

        let invalid_explicit_control = parse_experiment_definition(
            "name: ranking-v2\ndescription: Test ranking.\ngroups:\n  - name: control\n    weight: 0.5\n  - name: treatment\n    weight: 0.4\n",
        );
        assert!(invalid_explicit_control.is_err());
    }

    #[test]
    fn requires_login_for_account_requests() {
        let api = Api {
            client: Client::new(),
            endpoint: "https://api.statespace.com".into(),
            token: None,
        };
        let error = api.request(Method::GET, "/v1/projects").unwrap_err();
        assert_eq!(error.to_string(), "not logged in; run ssp login");
    }
}
