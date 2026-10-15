//! Command-line arguments.

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "ssp",
    version,
    about = "A/B test whole versions of your app in production",
    after_help = "Every command except `ssp proxy` and `ssp controller` prints JSON. \
                  Documentation: https://docs.statespace.com"
)]
pub struct Cli {
    /// Statespace API URL.
    #[arg(long, env = "STATESPACE_URL", global = true)]
    pub endpoint: Option<String>,
    /// A token. Defaults to the saved `ssp login` session.
    #[arg(long, env = "SSP_TOKEN", global = true, hide_env_values = true)]
    pub token: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Sign in with GitHub or Google and save the session.
    Login {
        /// Print the sign-in URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Sign this terminal out.
    Logout {
        /// Sign out every session of the account, everywhere.
        #[arg(long)]
        all: bool,
    },
    /// Show the account, the credential in use, what it may do, and storage use.
    Whoami,
    /// Print a new key pair for signing variants. Nothing is stored.
    Keygen,
    /// Manage tokens: credentials for programs, agents, and SQL.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Define apps: one codebase and deployment each.
    #[command(subcommand)]
    App(AppCommand),
    /// Publish signed images of an app as variants.
    #[command(subcommand)]
    Variant(VariantCommand),
    /// Compare variants against a control arm of the app's default.
    #[command(subcommand)]
    Experiment(ExperimentCommand),
    /// Run a reverse proxy that assigns each request's subject and forwards
    /// it to the app's default or to an experiment's arm.
    Proxy(Box<ProxyArgs>),
    /// Run the arms of an app's experiments on Kubernetes.
    Controller(Box<ControllerArgs>),
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Create a token. It is printed only once.
    Create {
        #[arg(long)]
        name: String,
        /// The usual scopes for a job.
        #[arg(long, conflicts_with = "scopes", required_unless_present = "scopes")]
        preset: Option<Preset>,
        /// A scope, for combinations no preset covers. Repeat for each.
        #[arg(long = "scope", value_parser = SCOPES)]
        scopes: Vec<String>,
        /// Limit what the token reads in SQL to one app. Needs `query:read`.
        #[arg(long, env = "STATESPACE_APP")]
        app: Option<String>,
        /// Days until the token expires. Omit for no expiration.
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=3650))]
        expires_in_days: Option<i64>,
    },
    /// List tokens and sessions, without their secrets.
    List,
    /// Revoke a token or session by its ID, or all of them.
    Revoke {
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        id: Option<String>,
        /// Revoke every token and session of the account.
        #[arg(long)]
        all: bool,
        /// Confirm revoking everything.
        #[arg(long, requires = "all")]
        yes: bool,
    },
}

pub const SCOPES: [&str; 6] = [
    "runtime:read",
    "events:write",
    "variants:report",
    "experiments:write",
    "query:read",
    "tokens:manage",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Preset {
    /// For `ssp proxy`: read the app's configuration and send events.
    Runtime,
    /// For `ssp controller`: runtime, and report where arms run.
    Controller,
    /// For CI: publish variants and run experiments.
    Publisher,
    /// For SQL: read runs, output, and requests.
    Analyst,
    /// Everything, for agents that run the whole loop.
    Admin,
}

impl Preset {
    pub fn scopes(self) -> Vec<&'static str> {
        match self {
            Self::Runtime => vec!["runtime:read", "events:write"],
            Self::Controller => vec!["runtime:read", "events:write", "variants:report"],
            Self::Publisher => vec!["experiments:write"],
            Self::Analyst => vec!["query:read"],
            Self::Admin => SCOPES.to_vec(),
        }
    }
}

#[derive(Subcommand)]
pub enum AppCommand {
    /// Create an app and its analysis schema.
    Create {
        /// Lowercase letters, digits, and underscores, starting with a letter.
        name: String,
        /// Days to keep the lines arms print. Defaults to 7.
        #[arg(long = "output-retention-days")]
        output_retention: Option<u32>,
    },
    /// Change an app's settings.
    Update {
        name: String,
        /// Days to keep the lines arms print.
        #[arg(long = "output-retention-days")]
        output_retention: Option<u32>,
    },
    /// List apps and the share of subjects their running experiments hold.
    List,
    /// Show an app.
    Show { name: String },
    /// Erase one subject's runs, requests, and lines in an app.
    EraseSubject { name: String, subject: String },
    /// Delete an app with its variants, experiments, and events.
    Delete {
        name: String,
        /// Confirm permanent deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum VariantCommand {
    /// Sign an image and publish it as the next version of a variant.
    Create {
        /// The variant's name, such as `lower-price`.
        name: String,
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
        /// The image, by tag or digest. A tag is resolved to its digest with
        /// `docker`, so the signature covers exact bytes.
        #[arg(long)]
        image: String,
        /// The commit the image was built from. Defaults to CI's commit, such
        /// as `GITHUB_SHA`, or the current directory's Git HEAD.
        #[arg(long)]
        commit: Option<String>,
        /// The base64 private key from `ssp keygen`.
        #[arg(long, env = "SSP_SIGNING_KEY", hide_env_values = true)]
        signing_key: String,
    },
    /// List the newest version of each variant of an app.
    List {
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
    },
    /// Show every version of a variant.
    Show {
        name: String,
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
    },
    /// Delete a variant that no experiment uses.
    Delete {
        name: String,
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum ExperimentCommand {
    /// Create an experiment as a draft.
    Create {
        name: String,
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
        /// CEL over `request` that yields the subject ID. Defaults to
        /// `request.cookies.ssp_sid`, a cookie the proxy sets. It cannot
        /// change after the experiment starts.
        #[arg(long)]
        subject: Option<String>,
        #[command(flatten)]
        settings: Settings,
    },
    /// Change the draft. The running version changes only on `start`.
    Update {
        name: String,
        /// CEL over `request` that yields the subject ID. Only before the first start.
        #[arg(long)]
        subject: Option<String>,
        #[command(flatten)]
        settings: Settings,
        /// Remove the eligibility rule.
        #[arg(long, conflicts_with = "eligibility")]
        no_eligibility: bool,
    },
    /// List experiments.
    List {
        #[arg(long, env = "STATESPACE_APP")]
        app: Option<String>,
    },
    /// Show an experiment's draft and its published version.
    Show { name: String },
    /// Publish the draft, pinning each variant's newest version, and run it.
    Start {
        name: String,
        /// Restart this published version instead.
        #[arg(long)]
        version: Option<u32>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Stop the running version. Its subjects get the default again.
    Stop {
        name: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Delete a stopped experiment with all of its versions and runs.
    Delete {
        name: String,
        /// Confirm permanent deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct Settings {
    /// An arm and its share of all the app's subjects, such as
    /// `lower-price=0.05`. Repeat for each variant. Control gets the share of
    /// the largest variant unless `control=SHARE` sets it. Replaces the
    /// draft's arms.
    #[arg(long = "split", value_name = "ARM=SHARE")]
    pub split: Vec<String>,
    /// CEL over `request`, such as `request.headers["cf-ipcountry"] == "US"`.
    #[arg(long)]
    pub eligibility: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// A note for the people and agents who read this experiment later.
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Runtime {
    /// Run the default and every arm as containers on this machine.
    Docker,
    /// Route only: Kubernetes runs the default, and `ssp controller` the arms.
    Kubernetes,
}

/// What `ssp proxy` and `ssp controller` share.
#[derive(Args)]
pub struct Shared {
    /// The app whose experiments to serve.
    #[arg(long, env = "STATESPACE_APP")]
    pub app: String,
    /// A path that must answer 200 before an arm gets traffic. Without it,
    /// an arm is ready once its port accepts connections.
    #[arg(long, env = "STATESPACE_HEALTH")]
    pub health: Option<String>,
    /// How many arms run at once, control arms included, 1 to 64.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
    pub max_variants: Option<u32>,
    /// Seconds to keep routing with the last configuration while Statespace
    /// is unreachable. After that, every subject gets the default.
    #[arg(long, default_value_t = 300)]
    pub stale_after: u64,
    /// An environment variable every arm gets and the default does not, such
    /// as a database URL without schema permissions. Repeat for each.
    #[arg(long = "variant-env", value_name = "NAME=VALUE")]
    pub variant_env: Vec<String>,
    /// A regular expression whose matches are replaced in recorded lines
    /// before they leave this machine. Repeat for each.
    #[arg(long = "redact", value_name = "REGEX")]
    pub redact: Vec<String>,
}

#[derive(Args)]
pub struct ProxyArgs {
    #[command(flatten)]
    pub shared: Shared,
    /// How arms run.
    #[arg(long, env = "STATESPACE_RUNTIME", value_enum)]
    pub runtime: Runtime,
    /// The public address.
    #[arg(long, env = "STATESPACE_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: String,
    /// The app's default image. With `--runtime docker`.
    #[arg(long, env = "STATESPACE_IMAGE", conflicts_with = "image_file")]
    pub image: Option<String>,
    /// A file holding the default image. On SIGHUP, the proxy reads it again
    /// and swaps the default without dropping requests. With `--runtime docker`.
    #[arg(long, env = "STATESPACE_IMAGE_FILE")]
    pub image_file: Option<std::path::PathBuf>,
    /// The port the app listens on in its container. Defaults to the
    /// image's `EXPOSE`. With `--runtime docker`.
    #[arg(long, env = "STATESPACE_PORT")]
    pub port: Option<u16>,
    /// A volume, as `docker run --volume` takes it. Repeat for each. With
    /// `--runtime docker`.
    #[arg(long = "volume", value_name = "SOURCE:TARGET")]
    pub volumes: Vec<String>,
    /// A memory limit for each container, such as `1g`. With `--runtime docker`.
    #[arg(long)]
    pub memory: Option<String>,
    /// A CPU limit for each container, such as `1.5`. With `--runtime docker`.
    #[arg(long)]
    pub cpus: Option<String>,
    /// Where the default listens, such as `http://localhost:8000`. With
    /// `--runtime kubernetes`.
    #[arg(long, env = "STATESPACE_DEFAULT")]
    pub default: Option<String>,
    /// A base64 public key whose signed variants may run. Repeat to rotate
    /// keys. With `--runtime docker`.
    #[arg(
        long = "trust-key",
        value_name = "KEY",
        env = "SSP_TRUST_KEY",
        value_delimiter = ','
    )]
    pub trust_keys: Vec<String>,
}

#[derive(Args)]
pub struct ControllerArgs {
    #[command(flatten)]
    pub shared: Shared,
    /// The Deployment that runs the app's default.
    #[arg(long, env = "STATESPACE_DEPLOYMENT")]
    pub deployment: String,
    /// The default's app container. Defaults to its first container other
    /// than `statespace-proxy`.
    #[arg(long, env = "STATESPACE_CONTAINER")]
    pub container: Option<String>,
    /// The namespace of the Deployment. Defaults to the controller's own.
    #[arg(long, env = "STATESPACE_NAMESPACE")]
    pub namespace: Option<String>,
    /// A base64 public key whose signed variants may run. Repeat to rotate keys.
    #[arg(
        long = "trust-key",
        value_name = "KEY",
        env = "SSP_TRUST_KEY",
        value_delimiter = ',',
        required = true
    )]
    pub trust_keys: Vec<String>,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_the_quickstart() {
        for arguments in [
            vec!["ssp", "app", "create", "web"],
            vec!["ssp", "keygen"],
            vec!["ssp", "whoami"],
            vec!["ssp", "logout", "--all"],
            vec![
                "ssp",
                "variant",
                "create",
                "lower-price",
                "--app",
                "web",
                "--image",
                "ghcr.io/acme/web:abc",
                "--signing-key",
                "k",
            ],
            vec![
                "ssp",
                "experiment",
                "create",
                "checkout",
                "--app",
                "web",
                "--split",
                "lower-price=0.05",
                "--note",
                "why",
            ],
            vec![
                "ssp",
                "experiment",
                "update",
                "checkout",
                "--no-eligibility",
            ],
            vec!["ssp", "experiment", "start", "checkout", "--note", "go"],
            vec!["ssp", "experiment", "stop", "checkout"],
            vec![
                "ssp",
                "proxy",
                "--app",
                "web",
                "--runtime",
                "docker",
                "--image",
                "ghcr.io/acme/web:main",
                "--trust-key",
                "k",
                "--variant-env",
                "A=b",
                "--memory",
                "1g",
            ],
            vec![
                "ssp",
                "proxy",
                "--app",
                "web",
                "--runtime",
                "kubernetes",
                "--default",
                "http://localhost:8000",
            ],
            vec![
                "ssp",
                "controller",
                "--app",
                "web",
                "--deployment",
                "web",
                "--trust-key",
                "k",
            ],
            vec![
                "ssp",
                "token",
                "create",
                "--name",
                "ci",
                "--preset",
                "publisher",
            ],
            vec![
                "ssp",
                "token",
                "create",
                "--name",
                "sql",
                "--scope",
                "query:read",
                "--app",
                "web",
            ],
            vec!["ssp", "token", "revoke", "--all", "--yes"],
        ] {
            Cli::try_parse_from(&arguments)
                .unwrap_or_else(|error| panic!("{arguments:?}: {error}"));
        }
    }

    #[test]
    fn rejects_unknown_scopes_and_runtimes() {
        for arguments in [
            vec![
                "ssp",
                "token",
                "create",
                "--name",
                "a",
                "--scope",
                "keys:manage",
            ],
            vec!["ssp", "proxy", "--app", "web", "--runtime", "local"],
            vec!["ssp", "controller", "--app", "web", "--deployment", "web"],
            vec!["ssp", "token", "revoke"],
        ] {
            assert!(Cli::try_parse_from(&arguments).is_err(), "{arguments:?}");
        }
    }
}
