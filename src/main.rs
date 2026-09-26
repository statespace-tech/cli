use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use reqwest::{Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod component;
mod toolchain;

#[derive(Parser)]
#[command(name = "ssp", version, about = "Manage Statespace experiments")]
struct Cli {
    #[arg(long, env = "STATESPACE_URL", global = true)]
    endpoint: Option<String>,
    #[arg(
        long = "api-key",
        env = "SSP_API_KEY",
        global = true,
        hide_env_values = true
    )]
    api_key: Option<String>,
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
    /// Manage API keys.
    Key(KeyCommand),
    /// Manage direct PostgreSQL access.
    Database(DatabaseCommand),
    /// Manage experiments and immutable versions.
    Experiment(ExperimentCommand),
    /// Build and publish executable functions.
    Function(FunctionCommand),
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
struct KeyCommand {
    #[command(subcommand)]
    command: KeySubcommand,
}

#[derive(Subcommand)]
enum KeySubcommand {
    /// Create a key and print its secret once.
    Create {
        #[arg(short = 'n', long)]
        name: String,
        #[arg(long, value_parser = ["runtime", "admin"], conflicts_with = "scopes", required_unless_present = "scopes")]
        preset: Option<String>,
        #[arg(long = "scope", value_parser = ["experiments:write", "functions:write", "runtime:read", "events:write", "query:read", "keys:manage"], required_unless_present = "preset")]
        scopes: Vec<String>,
    },
    /// List active keys without their secrets.
    List,
    /// Revoke a key.
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
    /// Create a draft experiment with a complete traffic split.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "variant", required = true)]
        variants: Vec<String>,
    },
    /// Replace the complete traffic split with a new immutable version.
    Update {
        #[arg(long)]
        name: String,
        #[arg(long = "variant", required = true)]
        variants: Vec<String>,
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
struct FunctionCommand {
    #[command(subcommand)]
    command: FunctionSubcommand,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum FunctionLanguage {
    Python,
    Typescript,
    Javascript,
    Go,
    Rust,
    C,
    Cpp,
}

#[derive(Subcommand)]
enum FunctionSubcommand {
    /// Build a function as a local WebAssembly file.
    Build {
        source: PathBuf,
        #[arg(long)]
        language: FunctionLanguage,
        #[arg(long)]
        entry: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Validate and publish a built function.
    Publish {
        artifact: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// List published function versions.
    List,
    /// Show one published function version.
    Show {
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: u32,
    },
    /// Delete an unreferenced function version.
    Delete {
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: u32,
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
struct ExperimentGroup {
    name: String,
    weight: f64,
    #[serde(default)]
    config: Value,
}

#[derive(Debug, Serialize)]
struct FunctionExperimentDefinition {
    name: String,
    variants: Vec<FunctionVariant>,
}

#[derive(Debug, Serialize)]
struct FunctionVariant {
    function: String,
    weight: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct FunctionView {
    name: String,
    version: u32,
    sha256: String,
    size: usize,
    created_at: String,
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
struct KeyView {
    id: String,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct KeySecret {
    #[serde(flatten)]
    details: KeyView,
    key: String,
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
        token: cli.api_key.clone().or_else(|| settings.token.clone()),
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
        Command::Key(command) => run_key(&api, command).await?,
        Command::Database(command) => run_database(&api, command).await?,
        Command::Experiment(command) => run_experiment(&api, command).await?,
        Command::Function(command) => run_function(&api, command).await?,
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

async fn run_key(api: &Api, command: KeyCommand) -> anyhow::Result<()> {
    match command.command {
        KeySubcommand::Create { name, preset, scopes } => print_json(
            &api.send::<KeySecret>(Method::POST, "/v1/keys", Some(json!({
                "name": name,
                "scopes": match preset.as_deref() {
                    Some("runtime") => vec!["runtime:read", "events:write"],
                    Some("admin") => vec!["experiments:write", "functions:write", "runtime:read", "events:write", "query:read", "keys:manage"],
                    _ => scopes.iter().map(String::as_str).collect(),
                },
            })))
                .await?,
        )?,
        KeySubcommand::List => print_json(&api.get::<Vec<KeyView>>("/v1/keys").await?)?,
        KeySubcommand::Revoke { id } => {
            api.delete(&format!("/v1/keys/{id}")).await?;
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
        ExperimentSubcommand::Create { name, variants } => {
            let definition = function_experiment(&name, &variants)?;
            print_json(
                &api.send::<ExperimentView>(
                    Method::POST,
                    "/v1/function-experiments",
                    Some(serde_json::to_value(definition)?),
                )
                .await?,
            )?;
        }
        ExperimentSubcommand::Update { name, variants } => {
            let definition = function_experiment(&name, &variants)?;
            let path = format!("/v1/function-experiments/{name}");
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

fn function_experiment(
    name: &str,
    variants: &[String],
) -> anyhow::Result<FunctionExperimentDefinition> {
    if name.is_empty() || name.len() > 100 {
        bail!("experiment name must contain 1 to 100 characters");
    }
    let mut parsed = Vec::with_capacity(variants.len());
    let mut total = 0.0;
    for variant in variants {
        let (function, weight) = variant
            .rsplit_once('=')
            .context("variant must use function@version=weight")?;
        let (function_name, selector) = function
            .rsplit_once('@')
            .context("variant must use function@version=weight")?;
        if function_name.is_empty()
            || !(selector == "latest" || selector.parse::<u32>().is_ok_and(|v| v > 0))
        {
            bail!("variant must use a function name and @latest or a positive version");
        }
        let weight: f64 = weight.parse().context("invalid variant weight")?;
        if !weight.is_finite() || weight <= 0.0 || weight > 1.0 {
            bail!("variant weight must be greater than zero and at most one");
        }
        total += weight;
        parsed.push(FunctionVariant {
            function: function.into(),
            weight,
        });
    }
    if total > 1.0 + 1e-12 {
        bail!("variant weights must total at most one");
    }
    Ok(FunctionExperimentDefinition {
        name: name.into(),
        variants: parsed,
    })
}

async fn run_function(api: &Api, command: FunctionCommand) -> anyhow::Result<()> {
    match command.command {
        FunctionSubcommand::Build {
            source,
            language,
            entry,
            output,
        } => {
            let build_source = source.clone();
            let build_output = output.clone();
            tokio::task::spawn_blocking(move || {
                component::build(&build_source, language, &entry, &build_output)
            })
            .await??;
            let artifact = component::inspect(&output)?;
            print_json(&json!({
                "artifact": output,
                "sha256": artifact.sha256,
                "size": artifact.size,
            }))?;
        }
        FunctionSubcommand::Publish {
            artifact,
            name,
            dry_run,
        } => {
            let checked = component::inspect(&artifact)?;
            if dry_run {
                print_json(&json!({
                    "name": name,
                    "artifact": artifact,
                    "sha256": checked.sha256,
                    "size": checked.size,
                    "status": "valid",
                }))?;
            } else {
                let path = format!("/v1/functions/{name}");
                print_json(
                    &api.send_bytes::<FunctionView>(Method::POST, &path, std::fs::read(&artifact)?)
                        .await?,
                )?;
            }
        }
        FunctionSubcommand::List => {
            print_json(&api.get::<Vec<FunctionView>>("/v1/functions").await?)?;
        }
        FunctionSubcommand::Show { name, version } => {
            print_json(
                &api.get::<FunctionView>(&format!("/v1/functions/{name}/{version}"))
                    .await?,
            )?;
        }
        FunctionSubcommand::Delete { name, version } => {
            api.delete(&format!("/v1/functions/{name}/{version}"))
                .await?;
            print_json(&json!({ "deleted": format!("{name}@{version}") }))?;
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
            .context("authentication required; run ssp login or set SSP_API_KEY")?;
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

    async fn send_bytes<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        bytes: Vec<u8>,
    ) -> anyhow::Result<T> {
        decode(
            self.request(method, path)?
                .header(reqwest::header::CONTENT_TYPE, "application/wasm")
                .body(bytes)
                .send()
                .await?,
        )
        .await
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
        return anyhow::anyhow!("credential expired or invalid; run ssp login or set SSP_API_KEY");
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
        Cli::try_parse_from([
            "ssp",
            "key",
            "create",
            "--name",
            "production",
            "--preset",
            "runtime",
        ])
        .unwrap();
        Cli::try_parse_from([
            "ssp",
            "database",
            "credential",
            "create",
            "--name",
            "analyst",
        ])
        .unwrap();
        Cli::try_parse_from([
            "ssp",
            "experiment",
            "create",
            "--name",
            "ranking",
            "--variant",
            "ranker@2=0.2",
        ])
        .unwrap();
        Cli::try_parse_from(["ssp", "experiment", "start", "--name", "rank-v2"]).unwrap();
        Cli::try_parse_from([
            "ssp",
            "function",
            "build",
            "./ranker",
            "--language",
            "python",
            "--entry",
            "ranker:execute",
            "--output",
            "ranker.wasm",
        ])
        .unwrap();
        for language in ["c", "cpp"] {
            Cli::try_parse_from([
                "ssp",
                "function",
                "build",
                "./ranker.c",
                "--language",
                language,
                "--entry",
                "score",
                "--output",
                "ranker.wasm",
            ])
            .unwrap();
        }
        Cli::try_parse_from([
            "ssp",
            "function",
            "build",
            "./ranker",
            "--language",
            "go",
            "--entry",
            ".:Score",
            "--output",
            "ranker.wasm",
        ])
        .unwrap();
        assert!(Cli::try_parse_from(["ssp", "component", "list"]).is_err());
        Cli::try_parse_from(["ssp", "query", "SELECT 1"]).unwrap();
    }

    #[test]
    fn validates_complete_traffic_snapshot() {
        let definition = function_experiment(
            "ranking",
            &["ranker@latest=0.2".into(), "other@2=0.1".into()],
        )
        .unwrap();
        assert_eq!(definition.variants.len(), 2);
        assert!(function_experiment("ranking", &["ranker=0.2".into()]).is_err());
        assert!(function_experiment("ranking", &["ranker@1=1.1".into()]).is_err());
    }
}
