//! `ssp`: run A/B tests on functions and values in production.
//!
//! Every command prints one JSON document on standard output. Errors print
//! `{"error": "..."}` on standard error and exit with status 1.

mod api;
mod build;
mod cli;
mod commands;
mod config;
mod params;
mod wasm;

use clap::Parser;
use serde_json::json;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(error) = commands::run(cli) {
        eprintln!("{}", json!({ "error": format!("{error:#}") }));
        std::process::exit(1);
    }
}
