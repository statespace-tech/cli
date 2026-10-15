//! Command handlers. Each prints one JSON document, except `ssp proxy` and
//! `ssp controller`, which run until they are stopped.

mod app;
mod auth;
mod experiment;
mod token;
mod variant;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    api::Api,
    cli::{Cli, Command},
    config::{DEFAULT_ENDPOINT, Settings},
    controller, proxy, signing,
};

pub fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    let endpoint = cli
        .endpoint
        .or_else(|| settings.endpoint.clone())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.into());
    let api = Api::new(&endpoint, cli.token.or(settings.token))?;
    match cli.command {
        Command::Login { no_open } => auth::login(&api, no_open),
        Command::Logout { all } => auth::logout(&api, all),
        Command::Whoami => print(&api.get::<Value>("/v1/whoami")?),
        Command::Keygen => {
            let (private_key, public_key) = signing::generate();
            print(&json!({ "private_key": private_key, "public_key": public_key }))
        }
        Command::Token(command) => token::run(&api, command),
        Command::App(command) => app::run(&api, command),
        Command::Variant(command) => variant::run(&api, command),
        Command::Experiment(command) => experiment::run(&api, command),
        Command::Proxy(arguments) => proxy::run(api, *arguments),
        Command::Controller(arguments) => controller::run(api, *arguments),
    }
}

pub fn print(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
