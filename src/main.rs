use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use reqwest::{Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "ssp", version, about = "Manage Statespace experiments")]
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
    /// Sign in with GitHub or Google.
    Login {
        /// Print the login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Remove the saved account session.
    Logout,
    /// Show the signed-in account.
    Account,
    /// Manage SDK and CI tokens.
    Token(TokenCommand),
    /// Manage direct PostgreSQL access.
    Database(DatabaseCommand),
    /// Manage experiments and immutable versions.
    Experiment(ExperimentCommand),
    /// Run one read-only PostgreSQL query.
    Query {
        /// A SELECT statement.
        sql: String,
    },
    #[command(hide = true)]
    /// Run internal service administration commands.
    Admin(AdminCommand),
}

#[derive(Args)]
struct TokenCommand {
    #[command(subcommand)]
    command: TokenSubcommand,
}

#[derive(Subcommand)]
enum TokenSubcommand {
    /// Create a token and print its secret once.
    Create {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// List active tokens without their secrets.
    List,
    /// Revoke a token.
    Revoke {
        #[arg(long)]
        id: String,
    },
}

#[derive(Args)]
struct DatabaseCommand {
    #[command(subcommand)]
    command: DatabaseSubcommand,
}

#[derive(Subcommand)]
enum DatabaseSubcommand {
    /// Manage read-only PostgreSQL credentials.
    Credential(DatabaseCredentialCommand),
}

#[derive(Args)]
struct DatabaseCredentialCommand {
    #[command(subcommand)]
    command: DatabaseCredentialSubcommand,
}

#[derive(Subcommand)]
enum DatabaseCredentialSubcommand {
    /// Create a credential and print its connection URL once.
    Create {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// List active credentials without passwords.
    List,
    /// Revoke a credential.
    Revoke {
        #[arg(long)]
        id: String,
    },
}

#[derive(Args)]
struct ExperimentCommand {
    #[command(subcommand)]
    command: ExperimentSubcommand,
}

#[derive(Subcommand)]
enum ExperimentSubcommand {
    /// Create version 1 as a draft from YAML.
    Create {
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Publish the next immutable draft version from YAML.
    Publish {
        #[arg(short, long)]
        file: PathBuf,
    },
    /// List the latest version of each experiment.
    List,
    /// Show the latest version of one experiment.
    Show {
        #[arg(short = 'n', long)]
        name: String,
    },
    /// Start a version and stop its previous running version.
    Start {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(long)]
        version: Option<u32>,
    },
    /// Stop a version.
    Stop {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(long)]
        version: Option<u32>,
    },
    /// Delete an experiment and its data.
    Delete {
        #[arg(short = 'n', long)]
        name: String,
    },
}

#[derive(Args)]
struct AdminCommand {
    #[command(subcommand)]
    command: AdminSubcommand,
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
struct AccountDetails {
    id: String,
    provider: String,
    login: String,
    plan: String,
    database_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LoginOutput {
    status: &'static str,
    account: AccountDetails,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExperimentDefinition {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_assignment")]
    assignment: String,
    #[serde(default)]
    eligibility: Option<String>,
    groups: Vec<ExperimentGroup>,
}

fn default_assignment() -> String {
    "subject_id".into()
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
    eligibility: Option<String>,
    version: u32,
    status: String,
    groups: Vec<ExperimentGroup>,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenView {
    id: String,
    name: String,
    prefix: String,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenSecret {
    #[serde(flatten)]
    details: TokenView,
    token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatabaseCredential {
    id: String,
    name: String,
    username: String,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatabaseCredentialSecret {
    #[serde(flatten)]
    details: DatabaseCredential,
    database_url: String,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    rows: Value,
}

struct Api {
    client: Client,
    endpoint: String,
    token: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}", json!({ "error": format!("{error:#}") }));
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
        client: Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("statespace-cli/", env!("CARGO_PKG_VERSION")))
            .build()?,
        endpoint: endpoint.clone(),
        token: cli.token.clone().or_else(|| settings.token.clone()),
    };

    match cli.command {
        Command::Login { no_open } => {
            let login = start_login(&api).await?;
            let printed_url = no_open || webbrowser::open(&login.verification_url).is_err();
            if printed_url {
                eprintln!("Login URL: {}", login.verification_url);
            } else {
                eprintln!("Complete sign-in in your browser.");
            }
            let complete = wait_for_login(&api, &login).await?;
            let authenticated = Api {
                client: api.client.clone(),
                endpoint: api.endpoint.clone(),
                token: Some(complete.token.clone()),
            };
            let account = authenticated.get::<AccountDetails>("/v1/account").await?;
            settings = Settings {
                endpoint: Some(endpoint),
                token: Some(complete.token),
            };
            settings.save()?;
            print_json(&LoginOutput {
                status: "authenticated",
                account,
            })?;
        }
        Command::Logout => {
            settings.endpoint = Some(endpoint);
            settings.token = None;
            settings.save()?;
            print_json(&json!({ "status": "logged_out" }))?;
        }
        Command::Account => print_json(&api.get::<AccountDetails>("/v1/account").await?)?,
        Command::Token(command) => run_token(&api, command).await?,
        Command::Database(command) => run_database(&api, command).await?,
        Command::Experiment(command) => run_experiment(&api, command).await?,
        Command::Query { sql } => {
            let response = api
                .send::<QueryResponse>(Method::POST, "/v1/query", Some(json!({ "sql": sql })))
                .await?;
            print_json(&response.rows)?;
        }
        Command::Admin(command) => run_admin(&api, command).await?,
    }
    Ok(())
}

async fn run_token(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command.command {
        TokenSubcommand::Create { name } => print_json(
            &api.send::<TokenSecret>(Method::POST, "/v1/tokens", Some(json!({ "name": name })))
                .await?,
        )?,
        TokenSubcommand::List => print_json(&api.get::<Vec<TokenView>>("/v1/tokens").await?)?,
        TokenSubcommand::Revoke { id } => {
            api.delete(&format!("/v1/tokens/{id}")).await?;
            print_json(&json!({ "revoked": id }))?;
        }
    }
    Ok(())
}

async fn run_database(api: &Api, command: DatabaseCommand) -> anyhow::Result<()> {
    match command.command {
        DatabaseSubcommand::Credential(command) => match command.command {
            DatabaseCredentialSubcommand::Create { name } => print_json(
                &api.send::<DatabaseCredentialSecret>(
                    Method::POST,
                    "/v1/database/credentials",
                    Some(json!({ "name": name })),
                )
                .await?,
            )?,
            DatabaseCredentialSubcommand::List => print_json(
                &api.get::<Vec<DatabaseCredential>>("/v1/database/credentials")
                    .await?,
            )?,
            DatabaseCredentialSubcommand::Revoke { id } => {
                api.delete(&format!("/v1/database/credentials/{id}"))
                    .await?;
                print_json(&json!({ "revoked": id }))?;
            }
        },
    }
    Ok(())
}

async fn run_experiment(api: &Api, command: ExperimentCommand) -> anyhow::Result<()> {
    match command.command {
        ExperimentSubcommand::Create { file } => {
            let definition = read_experiment(&file)?;
            print_json(
                &api.send::<ExperimentView>(
                    Method::POST,
                    "/v1/experiments",
                    Some(serde_json::to_value(definition)?),
                )
                .await?,
            )?;
        }
        ExperimentSubcommand::Publish { file } => {
            let definition = read_experiment(&file)?;
            let path = format!("/v1/experiments/{}", definition.name);
            print_json(
                &api.send::<ExperimentView>(
                    Method::PUT,
                    &path,
                    Some(serde_json::to_value(definition)?),
                )
                .await?,
            )?;
        }
        ExperimentSubcommand::List => {
            print_json(&api.get::<Vec<ExperimentView>>("/v1/experiments").await?)?;
        }
        ExperimentSubcommand::Show { name } => {
            print_json(
                &api.get::<ExperimentView>(&format!("/v1/experiments/{name}"))
                    .await?,
            )?;
        }
        ExperimentSubcommand::Start { name, version } => {
            set_experiment_status(api, &name, version, "running").await?;
        }
        ExperimentSubcommand::Stop { name, version } => {
            set_experiment_status(api, &name, version, "stopped").await?;
        }
        ExperimentSubcommand::Delete { name } => {
            api.delete(&format!("/v1/experiments/{name}")).await?;
            print_json(&json!({ "deleted": name }))?;
        }
    }
    Ok(())
}

async fn set_experiment_status(
    api: &Api,
    name: &str,
    version: Option<u32>,
    status: &str,
) -> anyhow::Result<()> {
    print_json(
        &api.send::<ExperimentView>(
            Method::POST,
            &format!("/v1/experiments/{name}/state"),
            Some(json!({ "status": status, "version": version })),
        )
        .await?,
    )
}

async fn run_admin(api: &Api, command: AdminCommand) -> anyhow::Result<()> {
    let admin_token =
        std::env::var("STATESPACE_ADMIN_TOKEN").context("STATESPACE_ADMIN_TOKEN is required")?;
    let admin = Api {
        client: api.client.clone(),
        endpoint: api.endpoint.clone(),
        token: Some(admin_token),
    };
    match command.command {
        AdminSubcommand::SetPlan { account, plan } => print_json(
            &admin
                .send::<AccountDetails>(
                    Method::PATCH,
                    &format!("/v1/admin/accounts/{account}"),
                    Some(json!({ "plan": plan })),
                )
                .await?,
        )?,
    }
    Ok(())
}

fn read_experiment(path: &PathBuf) -> anyhow::Result<ExperimentDefinition> {
    if !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yaml" | "yml")
    ) {
        bail!("experiment definition must use a .yaml or .yml file");
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read experiment file: {}", path.display()))?;
    let definition: ExperimentDefinition = serde_yaml_ng::from_str(&contents)
        .with_context(|| format!("invalid experiment file: {}", path.display()))?;
    validate_experiment(&definition)?;
    Ok(definition)
}

fn validate_experiment(definition: &ExperimentDefinition) -> anyhow::Result<()> {
    if definition.name.is_empty() || definition.name.len() > 100 {
        bail!("experiment name must contain 1 to 100 characters");
    }
    if definition.groups.is_empty() {
        bail!("experiment must contain at least one treatment group");
    }
    let mut total = 0.0;
    for group in &definition.groups {
        if group.name == "control" {
            bail!("control is implicit and must not appear in groups");
        }
        if !group.config.is_object() {
            bail!("each group config must be a JSON object");
        }
        total += group.weight;
    }
    if !total.is_finite() || total <= 0.0 || total >= 1.0 {
        bail!("treatment weights must total more than zero and less than one");
    }
    Ok(())
}

async fn start_login(api: &Api) -> anyhow::Result<LoginStart> {
    decode(
        api.client
            .post(format!("{}/v1/auth/device", api.endpoint))
            .send()
            .await?,
    )
    .await
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

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        decode(self.request(Method::GET, path)?.send().await?).await
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> anyhow::Result<T> {
        let mut request = self.request(method, path)?;
        if let Some(body) = body {
            request = request.json(&body);
        }
        decode(request.send().await?).await
    }

    async fn delete(&self, path: &str) -> anyhow::Result<()> {
        ensure_success(self.request(Method::DELETE, path)?.send().await?).await
    }
}

async fn decode<T: serde::de::DeserializeOwned>(response: Response) -> anyhow::Result<T> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(response_error(status, &bytes));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn ensure_success(response: Response) -> anyhow::Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let bytes = response.bytes().await?;
    Err(response_error(status, &bytes))
}

fn response_error(status: StatusCode, bytes: &[u8]) -> anyhow::Error {
    if status == StatusCode::UNAUTHORIZED {
        return anyhow::anyhow!("session expired or invalid; run ssp login");
    }
    let message = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned());
    anyhow::anyhow!("server returned {status}: {message}")
}

fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_commands() {
        Cli::try_parse_from(["ssp", "login"]).unwrap();
        Cli::try_parse_from(["ssp", "token", "create", "--name", "production"]).unwrap();
        Cli::try_parse_from([
            "ssp",
            "database",
            "credential",
            "create",
            "--name",
            "analyst",
        ])
        .unwrap();
        Cli::try_parse_from(["ssp", "experiment", "create", "--file", "experiment.yaml"]).unwrap();
        Cli::try_parse_from(["ssp", "experiment", "start", "--name", "rank-v2"]).unwrap();
        Cli::try_parse_from(["ssp", "query", "SELECT 1"]).unwrap();
    }

    #[test]
    fn validates_implicit_control() {
        let definition: ExperimentDefinition = serde_yaml_ng::from_str(
            "name: rank-v2\nassignment: user_id\ngroups:\n  - name: treatment\n    weight: 0.2\n    config: {reranker: rrf}\n",
        )
        .unwrap();
        validate_experiment(&definition).unwrap();
        assert!((definition.groups[0].weight - 0.2).abs() < f64::EPSILON);
    }
}
