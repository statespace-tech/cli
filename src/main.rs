use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
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
    #[arg(long, env = "STATESPACE_TOKEN", global = true, hide_env_values = true)]
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
    /// Manage projects.
    Project(ProjectCommand),
    /// Show the signed-in account and enforced plan limits.
    Account,
    /// Run service administration commands.
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
    /// Show one project and its experiments.
    Show {
        #[arg(short = 'N', long)]
        name: String,
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
    /// Replace a draft experiment from YAML.
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
    /// Delete an experiment and its assignments.
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
    token: String,
    api_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectDetails {
    id: String,
    url: String,
    token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProjectView {
    id: String,
    url: String,
    token: String,
    experiments: Vec<ExperimentView>,
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
    #[serde(default = "default_assignment_unit")]
    assignment_unit: String,
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
    assignment_unit: String,
    status: String,
    groups: Vec<ExperimentGroup>,
}

fn default_assignment_unit() -> String {
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
        let output = noyalib::to_string(&ErrorOutput {
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
    let api = Api {
        client: Client::new(),
        endpoint: endpoint.clone(),
        token: cli.token.clone().or_else(|| settings.token.clone()),
    };

    match cli.command {
        Command::Login { no_open } => {
            let login = start_login(&api).await?;
            let printed_url = no_open || webbrowser::open(&login.verification_url).is_err();
            if printed_url {
                print_yaml(&json!({ "login_url": login.verification_url }))?;
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
        },
        Command::Account => {
            let response = api.request(Method::GET, "/v1/account")?.send().await?;
            print_yaml(&decode::<AccountDetails>(response).await?)?;
        }
        Command::Admin(command) => match command.command {
            AdminSubcommand::SetPlan { account, plan } => {
                let response = api
                    .request(Method::PATCH, &format!("/v1/admin/accounts/{account}"))?
                    .json(&json!({ "plan": plan }))
                    .send()
                    .await?;
                print_yaml(&decode::<AccountDetails>(response).await?)?;
            }
        },
        Command::Experiment(command) => run_experiment(&api, command).await?,
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
    let definition: ExperimentDefinition = noyalib::from_str(contents)?;
    if definition.description.trim().is_empty() {
        bail!("experiment description must not be empty");
    }
    if definition.groups.is_empty() {
        bail!("experiment file must define at least one treatment group");
    }
    for group in &definition.groups {
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
    }
    Ok(definition)
}

fn print_yaml<T: Serialize + ?Sized>(value: &T) -> anyhow::Result<()> {
    let output = noyalib::to_string(value)?;
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
        assert_eq!(definition.assignment_unit, "subject_id");
        assert_eq!(definition.groups[0].config["model"], "gpt-5");
    }

    #[test]
    fn requires_an_experiment_description() {
        let result = parse_experiment_definition(
            "name: ranking-v2\ngroups:\n  - name: treatment\n    weight: 0.5\n",
        );
        assert!(result.is_err());
    }
}
