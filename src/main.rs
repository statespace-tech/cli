//! `ssp`: A/B test whole versions of an app in production.
//!
//! Every command except `ssp proxy` and `ssp controller` prints one JSON
//! document on standard output. Errors print `{"error": "..."}` on standard
//! error and exit with status 1.

mod api;
mod cli;
mod commands;
mod config;
mod controller;
mod image;
mod proxy;
mod signing;

use clap::Parser;
use serde_json::json;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(error) = commands::run(cli) {
        eprintln!("{}", json!({ "error": format!("{error:#}") }));
        std::process::exit(1);
    }
}
