//! Command-line arguments.

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "ssp",
    version,
    about = "Run A/B tests on functions and values in production",
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
    /// Define reusable groups of parameter values.
    #[command(subcommand)]
    Group(GroupCommand),
    /// Compare groups against the application's defaults.
    #[command(subcommand)]
    Experiment(ExperimentCommand),
    /// Build a function from source and run it locally in the SDK sandbox.
    Run {
        /// A function's source: `rerank.py:score`, `bm25.ts:rank`, or `./ranker:Score`.
        function: String,
        /// JSON input for the function.
        #[arg(long)]
        input: String,
        /// Execution timeout in milliseconds.
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
    },
    /// Manage API keys for applications and agents.
    #[command(subcommand)]
    Key(KeyCommand),
    /// Manage signed-in terminal sessions.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Manage read-only PostgreSQL access to runs and outcomes.
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

const PARAMETERS: &str = "Parameters as NAME=VALUE. A value is a function's source, such \
as rerank.py:score, bm25.ts:rank, or ./ranker:Score; a JSON file; a text file; or a JSON \
literal such as 20, true, or '\"text\"'.";

#[derive(Subcommand)]
pub enum GroupCommand {
    /// Create a group.
    Create {
        name: String,
        #[arg(required = true, value_name = "NAME=VALUE", help = PARAMETERS)]
        parameters: Vec<String>,
    },
    /// Change parameters, creating the next version. `NAME=` removes a parameter.
    Update {
        name: String,
        #[arg(required = true, value_name = "NAME=VALUE", help = PARAMETERS)]
        parameters: Vec<String>,
    },
    /// List the newest version of each group.
    List,
    /// Show the newest version of a group.
    Show { name: String },
    /// Delete a group that no experiment uses.
    Delete { name: String },
}

#[derive(Subcommand)]
pub enum ExperimentCommand {
    /// Create an experiment as a draft.
    Create {
        name: String,
        #[command(flatten)]
        settings: Settings,
    },
    /// Change the draft. The running version changes only on `start`.
    Update {
        name: String,
        #[command(flatten)]
        settings: Settings,
        /// Remove the eligibility rule.
        #[arg(long, conflicts_with = "eligibility")]
        no_eligibility: bool,
    },
    /// List experiments.
    List,
    /// Show an experiment's draft and its published version.
    Show { name: String },
    /// Publish the draft, pinning each group's newest version, and run it.
    Start {
        name: String,
        /// Restart this published version instead.
        #[arg(long)]
        version: Option<u32>,
    },
    /// Stop the running version. SDKs return the application's defaults.
    Stop { name: String },
    /// Compare an outcome across groups.
    Results {
        name: String,
        /// Outcome name logged by the SDK.
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
    /// Erase one subject's runs and outcomes.
    EraseSubject { name: String, subject: String },
    /// Delete an experiment with all of its versions, runs, and outcomes.
    Delete {
        name: String,
        /// Confirm permanent deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct Settings {
    /// A group and its share of eligible subjects, such as `bm25=0.2`. Repeat
    /// for each group; control receives the rest. Replaces the draft's groups.
    #[arg(long = "group", value_name = "NAME=WEIGHT")]
    pub groups: Vec<String>,
    /// CEL expression over the SDK `context`, such as `context.country == "US"`.
    #[arg(long)]
    pub eligibility: Option<String>,
    /// Label for the subject identifier passed to the SDK.
    #[arg(long)]
    pub assignment: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
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
            vec![
                "ssp",
                "group",
                "create",
                "bm25",
                "ranker=rerank.py:score",
                "top_k=50",
            ],
            vec!["ssp", "group", "update", "bm25", "top_k="],
            vec![
                "ssp",
                "experiment",
                "create",
                "ranking",
                "--group",
                "bm25=0.2",
            ],
            vec!["ssp", "experiment", "update", "ranking", "--no-eligibility"],
            vec!["ssp", "experiment", "start", "ranking"],
            vec![
                "ssp",
                "experiment",
                "results",
                "ranking",
                "--outcome",
                "click",
            ],
            vec!["ssp", "run", "rerank.py:score", "--input", "[3,1,2]"],
            vec![
                "ssp", "key", "create", "--name", "app", "--preset", "runtime",
            ],
        ] {
            Cli::try_parse_from(arguments).unwrap();
        }
    }

    #[test]
    fn groups_need_parameters() {
        assert!(Cli::try_parse_from(["ssp", "group", "create", "bm25"]).is_err());
    }
}
