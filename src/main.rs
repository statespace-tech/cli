use std::{
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
    /// Manage runtime and database tokens.
    Token(TokenCommand),
    /// Show the signed-in account and enforced plan limits.
    Account,
    #[command(hide = true)]
    /// Run internal service administration commands.
    Admin(AdminCommand),
    /// Manage experiments.
    Experiment(ExperimentCommand),
}

#[derive(Args)]
struct AdminCommand {
    #[command(subcommand)]
    command: AdminSubcommand,
}

#[derive(Args)]
struct TokenCommand {
    #[command(subcommand)]
    command: TokenSubcommand,
}

#[derive(Subcommand)]
enum TokenSubcommand {
    /// Create a database token and print its secret once.
    Create {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(long)]
        access: TokenAccess,
    },
    /// List active database tokens without their secrets.
    List,
    /// Revoke a database token.
    Revoke {
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
    /// List experiments.
    List,
    /// Show one experiment.
    Show {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// Start or restart an experiment.
    Start {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// Stop an experiment.
    Stop {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// Delete an experiment and its recorded data.
    Delete {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// Change live experiment traffic.
    Traffic(ExperimentTrafficCommand),
    /// Manage exclusion layers.
    Layer(ExperimentLayerCommand),
}

#[derive(Args)]
struct ExperimentTrafficCommand {
    #[command(subcommand)]
    command: ExperimentTrafficSubcommand,
}

#[derive(Subcommand)]
enum ExperimentTrafficSubcommand {
    /// Increase traffic for a running experiment.
    Set {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(long)]
        traffic: f64,
    },
}

#[derive(Args)]
struct ExperimentLayerCommand {
    #[command(subcommand)]
    command: ExperimentLayerSubcommand,
}

#[derive(Subcommand)]
enum ExperimentLayerSubcommand {
    /// Create an exclusion layer from YAML.
    Create {
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Replace an exclusion layer from YAML.
    Update {
        #[arg(short, long)]
        file: PathBuf,
    },
    /// List exclusion layers.
    List,
    /// Show one exclusion layer.
    Show {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// Delete an unused exclusion layer.
    Delete {
        #[arg(short = 'n', long)]
        name: String,
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
}

#[derive(Debug, Deserialize, Serialize)]
struct LoginOutput {
    status: &'static str,
    account: AccountDetails,
}

#[derive(Serialize)]
struct ErrorOutput {
    error: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum TokenAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatabaseToken {
    id: String,
    name: String,
    access: TokenAccess,
    prefix: String,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatabaseTokenSecret {
    #[serde(flatten)]
    details: DatabaseToken,
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
    database_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
enum AssignmentDefinition {
    Unit(String),
    Advanced(AssignmentConfig),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AssignmentConfig {
    unit: String,
    #[serde(default)]
    method: AssignmentMethod,
    #[serde(default = "default_block_size")]
    block_size: u32,
    #[serde(default)]
    strata: Vec<String>,
}

fn default_block_size() -> u32 {
    4
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum AssignmentMethod {
    #[default]
    Random,
    Stratified,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ExperimentAllocation {
    #[serde(default = "default_traffic")]
    traffic: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<String>,
}

impl Default for ExperimentAllocation {
    fn default() -> Self {
        Self {
            traffic: default_traffic(),
            layer: None,
        }
    }
}

fn default_traffic() -> f64 {
    1.0
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
    assignment: AssignmentDefinition,
    allocation: ExperimentAllocation,
    status: String,
    groups: Vec<ExperimentGroup>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LayerDefinition {
    name: String,
    description: String,
    #[serde(default = "default_layer_assignment")]
    assignment: String,
    #[serde(default)]
    holdout: f64,
}

fn default_layer_assignment() -> String {
    "subject_id".into()
}

#[derive(Debug, Deserialize, Serialize)]
struct ExperimentLayer {
    name: String,
    description: String,
    assignment: String,
    holdout: f64,
    experiments: Vec<String>,
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
            let authenticated_api = Api {
                client: api.client.clone(),
                endpoint: api.endpoint.clone(),
                token: Some(complete.token.clone()),
            };
            let account = decode::<AccountDetails>(
                authenticated_api
                    .request(Method::GET, "/v1/account")?
                    .send()
                    .await?,
            )
            .await?;
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
                account,
            })?;
        }
        Command::Logout => {
            settings.endpoint = Some(endpoint);
            settings.token = None;
            settings.save()?;
            print_yaml(&json!({ "status": "logged-out" }))?;
        }
        Command::Token(command) => run_token(&api, command).await?,
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

async fn run_token(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command.command {
        TokenSubcommand::Create { name, access } => {
            let response = api
                .request(Method::POST, "/v1/tokens")?
                .json(&json!({ "name": name, "access": access }))
                .send()
                .await?;
            print_yaml(&decode::<DatabaseTokenSecret>(response).await?)?;
        }
        TokenSubcommand::List => {
            let response = api.request(Method::GET, "/v1/tokens")?.send().await?;
            print_yaml(&decode::<Vec<DatabaseToken>>(response).await?)?;
        }
        TokenSubcommand::Revoke { id } => {
            let response = api
                .request(Method::DELETE, &format!("/v1/tokens/{id}"))?
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
        ExperimentSubcommand::List => {
            let response = api.request(Method::GET, "/v1/experiments")?.send().await?;
            print_yaml(&decode::<Vec<ExperimentView>>(response).await?)?;
        }
        ExperimentSubcommand::Show { name } => {
            let response = api
                .request(Method::GET, &format!("/v1/experiments/{name}"))?
                .send()
                .await?;
            print_yaml(&decode::<ExperimentView>(response).await?)?;
        }
        ExperimentSubcommand::Start { name } => {
            set_experiment_status(api, &name, "running").await?;
        }
        ExperimentSubcommand::Stop { name } => {
            set_experiment_status(api, &name, "stopped").await?;
        }
        ExperimentSubcommand::Delete { name } => {
            let response = api
                .request(Method::DELETE, &format!("/v1/experiments/{name}"))?
                .send()
                .await?;
            ensure_success(response).await?;
            print_yaml(&json!({ "deleted": true, "experiment": name }))?;
        }
        ExperimentSubcommand::Traffic(command) => match command.command {
            ExperimentTrafficSubcommand::Set { name, traffic } => {
                if !traffic.is_finite() || traffic <= 0.0 || traffic > 1.0 {
                    bail!("traffic must be greater than zero and no more than one");
                }
                let response = api
                    .request(Method::POST, &format!("/v1/experiments/{name}/traffic"))?
                    .json(&json!({ "traffic": traffic }))
                    .send()
                    .await?;
                print_yaml(&decode::<ExperimentView>(response).await?)?;
            }
        },
        ExperimentSubcommand::Layer(command) => run_experiment_layer(api, command).await?,
    }
    Ok(())
}

async fn run_experiment_layer(api: &Api, command: ExperimentLayerCommand) -> anyhow::Result<()> {
    match command.command {
        ExperimentLayerSubcommand::Create { file } => {
            let definition = read_layer_definition(&file)?;
            let response = api
                .request(Method::POST, "/v1/layers")?
                .json(&definition)
                .send()
                .await?;
            print_yaml(&decode::<ExperimentLayer>(response).await?)?;
        }
        ExperimentLayerSubcommand::Update { file } => {
            let definition = read_layer_definition(&file)?;
            let response = api
                .request(Method::PUT, &format!("/v1/layers/{}", definition.name))?
                .json(&definition)
                .send()
                .await?;
            print_yaml(&decode::<ExperimentLayer>(response).await?)?;
        }
        ExperimentLayerSubcommand::List => {
            let response = api.request(Method::GET, "/v1/layers")?.send().await?;
            print_yaml(&decode::<Vec<ExperimentLayer>>(response).await?)?;
        }
        ExperimentLayerSubcommand::Show { name } => {
            let response = api
                .request(Method::GET, &format!("/v1/layers/{name}"))?
                .send()
                .await?;
            print_yaml(&decode::<ExperimentLayer>(response).await?)?;
        }
        ExperimentLayerSubcommand::Delete { name } => {
            let response = api
                .request(Method::DELETE, &format!("/v1/layers/{name}"))?
                .send()
                .await?;
            ensure_success(response).await?;
            print_yaml(&json!({ "deleted": true, "layer": name }))?;
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

async fn set_experiment_status(api: &Api, name: &str, status: &str) -> anyhow::Result<()> {
    let response = api
        .request(Method::POST, &format!("/v1/experiments/{name}/state"))?
        .json(&json!({ "status": status }))
        .send()
        .await?;
    print_yaml(&decode::<ExperimentView>(response).await?)
}

fn read_layer_definition(path: &Path) -> anyhow::Result<LayerDefinition> {
    if !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yaml" | "yml")
    ) {
        bail!("layer definition must use a .yaml or .yml file");
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read layer file: {}", path.display()))?;
    parse_layer_definition(&contents)
        .with_context(|| format!("invalid layer file: {}", path.display()))
}

fn parse_layer_definition(contents: &str) -> anyhow::Result<LayerDefinition> {
    let definition: LayerDefinition = serde_yaml_ng::from_str(contents)?;
    if definition.name.trim().is_empty() {
        bail!("layer name must not be empty");
    }
    if definition.description.trim().is_empty() {
        bail!("layer description must not be empty");
    }
    if definition.assignment.trim().is_empty() {
        bail!("layer assignment must not be empty");
    }
    if !definition.holdout.is_finite() || !(0.0..1.0).contains(&definition.holdout) {
        bail!("layer holdout must be at least zero and less than one");
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
    fn parses_account_token_and_short_options() {
        assert!(matches!(
            Cli::try_parse_from(["ssp", "login"]).unwrap().command,
            Command::Login { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["ssp", "logout"]).unwrap().command,
            Command::Logout
        ));
        assert!(Cli::try_parse_from(["ssp", "entity", "show"]).is_err());
        Cli::try_parse_from([
            "ssp",
            "token",
            "create",
            "-n",
            "production",
            "--access",
            "read-write",
        ])
        .unwrap();
        Cli::try_parse_from(["ssp", "token", "revoke", "--id", "tok_123"]).unwrap();
        assert!(
            Cli::try_parse_from([
                "ssp",
                "token",
                "create",
                "-N",
                "production",
                "--access",
                "read-write",
            ])
            .is_err()
        );
    }

    #[test]
    fn experiments_need_no_entity_option() {
        Cli::try_parse_from(["ssp", "experiment", "list"]).unwrap();
        Cli::try_parse_from(["ssp", "experiment", "start", "-n", "rank-v2"]).unwrap();
        Cli::try_parse_from(["ssp", "experiment", "show", "-n", "rank-v2"]).unwrap();
        assert!(Cli::try_parse_from(["ssp", "--entity", "acme", "experiment", "list"]).is_err());
        assert!(Cli::try_parse_from(["ssp", "experiment", "create"]).is_err());
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
    fn parses_layer_and_advanced_commands() {
        let layer = parse_layer_definition(
            "name: search-ranking\ndescription: Ranking experiments.\nassignment: subject_id\nholdout: 0.05\n",
        )
        .unwrap();
        assert_eq!(layer.holdout, 0.05);

        Cli::try_parse_from([
            "ssp",
            "experiment",
            "traffic",
            "set",
            "-n",
            "rank-v2",
            "--traffic",
            "0.1",
        ])
        .unwrap();
        Cli::try_parse_from([
            "ssp",
            "experiment",
            "layer",
            "create",
            "--file",
            "layer.yaml",
        ])
        .unwrap();
    }

    #[test]
    fn requires_login_for_account_requests() {
        let api = Api {
            client: Client::new(),
            endpoint: "https://api.statespace.com".into(),
            token: None,
        };
        let error = api.request(Method::GET, "/v1/account").unwrap_err();
        assert_eq!(error.to_string(), "not logged in; run ssp login");
    }
}
