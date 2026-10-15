//! Variants: signed images of an app.

use std::process::Command;

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use super::{app, print};
use crate::{
    api::{Api, segment},
    cli::VariantCommand,
    image, signing,
};

pub fn run(api: &Api, command: VariantCommand) -> anyhow::Result<()> {
    match command {
        VariantCommand::Create {
            name,
            app,
            image,
            commit,
            signing_key,
        } => create(api, &app, &name, &image, commit, &signing_key),
        VariantCommand::List { app } => print(&api.get::<Value>(&variants(&app))?),
        VariantCommand::Show { name, app } => {
            print(&api.get::<Value>(&format!("{}/{}", variants(&app), segment(&name)))?)
        }
        VariantCommand::Delete { name, app, yes } => {
            if !yes {
                bail!("pass --yes to delete every version of {name}");
            }
            api.delete(&format!("{}/{}", variants(&app), segment(&name)))?;
            print(&json!({ "deleted": name, "app": app }))
        }
    }
}

fn variants(app: &str) -> String {
    format!("{}/variants", app::path(app))
}

/// Resolve the image to its digest, sign it, and publish it.
fn create(
    api: &Api,
    app: &str,
    name: &str,
    reference: &str,
    commit: Option<String>,
    signing_key: &str,
) -> anyhow::Result<()> {
    let key = signing::private_key(signing_key)?;
    let image = image::resolve(reference)?;
    let commit = commit
        .filter(|commit| !commit.is_empty())
        .or_else(current_commit);
    let message = signing::message(app, name, &image, commit.as_deref());
    let (signature, public_key) = signing::sign(&key, &message);
    let body = json!({
        "name": name,
        "image": image,
        "commit": commit,
        "signature": signature,
        "public_key": public_key,
    });
    print(&api.send::<Value>(Method::POST, &variants(app), &body)?)
}

/// The commit being built: from CI's environment, else the Git checkout
/// in the current directory.
fn current_commit() -> Option<String> {
    for variable in [
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "BUILDKITE_COMMIT",
        "CIRCLE_SHA1",
    ] {
        if let Ok(commit) = std::env::var(variable)
            && !commit.is_empty()
        {
            return Some(commit);
        }
    }
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !commit.is_empty()).then_some(commit)
}
