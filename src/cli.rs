//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "ssp",
    version,
    about = "Run A/B tests in production",
    after_help = "Every command prints JSON. Documentation: https://docs.statespace.com"
)]
pub struct Cli {
    /// Statespace API URL.
    #[arg(long, env = "STATESPACE_URL", global = true)]
    pub endpoint: Option<String>,
    /// API key. Defaults to the saved `ssp login` session.
    #[arg(long, env = "SSP_API_KEY", global = true, hide_env_values = true)]
    pub api_key: Option<String>,
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
    /// Remove the saved session.
    Logout,
    /// Show the signed-in account.
    Account,
    /// Define apps: one codebase and deployment each.
    #[command(subcommand)]
    App(AppCommand),
    /// Publish signed, ready-to-run builds of an app.
    #[command(subcommand)]
    Variant(VariantCommand),
    /// Compare variants against the deployed version of an app.
    #[command(subcommand)]
    Experiment(ExperimentCommand),
    /// Manage API keys for applications and agents.
    #[command(subcommand)]
    Key(KeyCommand),
    /// Manage signed-in terminal sessions.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Manage read-only PostgreSQL access to runs, outcomes, and logs.
    #[command(subcommand)]
    Database(DatabaseCommand),
    /// Show event storage usage and the account quota.
    Storage,
    /// List account audit records, newest first.
    Audit {
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Return records older than this ID.
        #[arg(long)]
        before: Option<i64>,
    },
}

#[derive(Subcommand)]
pub enum AppCommand {
    /// Create an app and its analysis schema.
    Create {
        /// Lowercase letters, digits, and underscores, starting with a letter.
        name: String,
        /// Days to keep captured output. Defaults to 7.
        #[arg(long)]
        output_retention: Option<i32>,
    },
    /// Change an app's settings.
    Update {
        name: String,
        /// Days to keep captured output.
        #[arg(long, required = true)]
        output_retention: Option<i32>,
    },
    /// List apps and the traffic their running experiments hold.
    List,
    /// Show an app.
    Show { name: String },
    /// List the hosts that recently reported for an app.
    Hosts { name: String },
    /// Erase one subject's runs, outcomes, and requests in an app.
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
    /// Print a new signing key pair. Keep the private key in CI.
    Keygen,
    /// Package, sign, and upload a ready-to-run build as the next version of a variant.
    Create {
        /// The variant's name, such as `feature-x`.
        name: String,
        /// The directory to package: the build as it runs, dependencies included.
        path: PathBuf,
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
        /// How to start this variant, with `{port}` for its port. Defaults to the host's command.
        #[arg(long)]
        command: Option<String>,
        /// The commit the build came from. Defaults to the directory's Git HEAD.
        #[arg(long)]
        commit: Option<String>,
        /// The base64 private key from `ssp variant keygen`.
        #[arg(long, env = "SSP_SIGNING_KEY", hide_env_values = true)]
        sign_key: String,
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
    /// Show a variant's newest lifecycle events and captured output.
    Logs {
        name: String,
        #[arg(long, env = "STATESPACE_APP")]
        app: String,
        #[arg(long)]
        version: Option<u32>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
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
        /// `request.cookies.ssp_sid`, a cookie the host sets. It cannot
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
    /// Stop the running version. Hosts route its subjects to control.
    Stop {
        name: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Compare an outcome across variants.
    Results {
        name: String,
        /// Outcome name the app prints.
        #[arg(long)]
        outcome: String,
        /// Numeric or boolean field to sum per subject. Omit for the share
        /// of subjects with the outcome.
        #[arg(long)]
        metric: Option<String>,
        /// Version to analyze. Defaults to the running or newest version.
        #[arg(long)]
        version: Option<u32>,
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
    /// A variant and its share of the experiment's subjects, such as
    /// `feature-x=0.5`. Repeat for each variant; control receives the rest.
    /// Replaces the draft's variants.
    #[arg(long = "variant", value_name = "NAME=WEIGHT")]
    pub variants: Vec<String>,
    /// The share of the app's subjects the experiment holds while it runs.
    /// Running experiments of an app never share subjects. Defaults to 1.
    #[arg(long)]
    pub traffic: Option<f64>,
    /// CEL over `request`, such as `request.headers["cf-ipcountry"] == "US"`.
    #[arg(long)]
    pub eligibility: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// A note for the people and agents who read this experiment later.
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Subcommand)]
pub enum KeyCommand {
    /// Create a key. Its secret is printed only once.
    Create {
        #[arg(long)]
        name: String,
        /// `runtime` for applications; `admin` for agents and automation.
        #[arg(long, conflicts_with = "scopes", required_unless_present = "scopes")]
        preset: Option<KeyPreset>,
        #[arg(long = "scope", value_parser = SCOPES)]
        scopes: Vec<String>,
        /// Days until the key expires. Omit for no expiration.
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        expires_in_days: Option<i64>,
    },
    /// List keys without their secrets.
    List,
    /// Revoke a key.
    Revoke { id: String },
}

pub const SCOPES: [&str; 6] = [
    "experiments:write",
    "runtime:read",
    "events:write",
    "query:read",
    "keys:manage",
    "audit:read",
];

#[derive(Clone, Copy, ValueEnum)]
pub enum KeyPreset {
    Runtime,
    Admin,
}

impl KeyPreset {
    pub fn scopes(self) -> Vec<&'static str> {
        match self {
            Self::Runtime => vec!["runtime:read", "events:write"],
            Self::Admin => SCOPES.to_vec(),
        }
    }
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// List terminal sessions.
    List,
    /// Revoke a terminal session.
    Revoke { id: String },
    /// Revoke every key, terminal session, and browser session.
    RevokeAll {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum DatabaseCommand {
    /// Manage read-only PostgreSQL credentials.
    #[command(subcommand)]
    Credential(CredentialCommand),
}

#[derive(Subcommand)]
pub enum CredentialCommand {
    /// Create a credential. Its connection URL is printed only once.
    Create {
        #[arg(long)]
        name: String,
        /// Limit the credential to one app's schema. Omit for every app.
        #[arg(long)]
        app: Option<String>,
    },
    /// List credentials without passwords.
    List,
    /// Revoke a credential and close its connections.
    Revoke { id: String },
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
            vec!["ssp", "variant", "keygen"],
            vec![
                "ssp",
                "variant",
                "create",
                "feature-x",
                "./build",
                "--app",
                "web",
                "--sign-key",
                "k",
            ],
            vec![
                "ssp",
                "experiment",
                "create",
                "checkout",
                "--app",
                "web",
                "--variant",
                "feature-x=0.5",
                "--traffic",
                "0.2",
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
                "experiment",
                "results",
                "checkout",
                "--outcome",
                "purchase",
            ],
            vec![
                "ssp", "key", "create", "--name", "app", "--preset", "runtime",
            ],
            vec![
                "ssp",
                "database",
                "credential",
                "create",
                "--name",
                "a",
                "--app",
                "web",
            ],
        ] {
            Cli::try_parse_from(arguments).unwrap();
        }
    }
}
