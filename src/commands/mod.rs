//! Command handlers. Each prints one JSON document.

mod access;
mod auth;
mod experiment;
mod group;

use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    api::Api,
    cli::{Cli, Command},
    config::{DEFAULT_ENDPOINT, Settings},
    params, wasm,
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
        Command::Group(command) => group::run(&api, command),
        Command::Experiment(command) => experiment::run(&api, command),
        Command::Run {
            function,
            input,
            timeout_ms,
        } => run_function(&function, &input, timeout_ms),
        Command::Key(command) => access::key(&api, command),
        Command::Token(command) => access::token(&api, command),
        Command::Database(command) => access::database(&api, command),
        Command::Storage => print(&api.get::<Value>("/v1/storage")?),
        Command::Audit { limit, before } => access::audit(&api, limit, before),
    }
}

/// Build a function if needed and run it once locally.
fn run_function(function: &str, input: &str, timeout_ms: u64) -> anyhow::Result<()> {
    let input: Value = serde_json::from_str(input).context("--input must be JSON")?;
    let reference = params::function_reference(function)
        .with_context(|| format!("{function} is not an existing file or directory"))?;
    let component = params::prepare(&reference)?;
    let execution = wasm::execute(&component.bytes, &input, Duration::from_millis(timeout_ms))?;
    print(&json!({ "output": execution.output, "duration_ms": execution.duration_ms }))
}

pub fn print(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
