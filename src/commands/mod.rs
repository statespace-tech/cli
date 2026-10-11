//! Command handlers. Each prints one JSON document.

mod access;
mod app;
mod auth;
mod experiment;
mod variant;

use serde::Serialize;
use serde_json::Value;

use crate::{
    api::Api,
    cli::{Cli, Command},
    config::{DEFAULT_ENDPOINT, Settings},
};

pub fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    let endpoint = cli
        .endpoint
        .or_else(|| settings.endpoint.clone())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.into());
    let api = Api::new(&endpoint, cli.api_key.or(settings.token))?;
    match cli.command {
        Command::Login { no_open } => auth::login(&api, no_open),
        Command::Logout => auth::logout(&api),
        Command::Account => print(&api.get::<Value>("/v1/account")?),
        Command::App(command) => app::run(&api, command),
        Command::Variant(command) => variant::run(&api, command),
        Command::Experiment(command) => experiment::run(&api, command),
        Command::Key(command) => access::key(&api, command),
        Command::Token(command) => access::token(&api, command),
        Command::Database(command) => access::database(&api, command),
        Command::Storage => print(&api.get::<Value>("/v1/storage")?),
        Command::Audit { limit, before } => access::audit(&api, limit, before),
    }
}

pub fn print(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
