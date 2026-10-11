//! Variants: signed, ready-to-run builds of an app.

use std::{collections::BTreeMap, fs::File, path::Path, process::Command, time::Duration};

use anyhow::{Context, bail};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{app, print};
use crate::{
    api::{Api, segment},
    bundle,
    cli::VariantCommand,
    signing,
};

#[derive(Deserialize)]
struct Upload {
    exists: bool,
    upload_url: Option<String>,
    #[serde(default)]
    upload_headers: BTreeMap<String, String>,
}

pub fn run(api: &Api, command: VariantCommand) -> anyhow::Result<()> {
    match command {
        VariantCommand::Keygen => {
            let (private_key, public_key) = signing::generate();
            print(&json!({ "private_key": private_key, "public_key": public_key }))
        }
        VariantCommand::Create {
            name,
            path,
            app,
            command,
            commit,
            sign_key,
        } => create(api, &app, &name, &path, command, commit, &sign_key),
        VariantCommand::List { app } => print(&api.get::<Value>(&variants(&app))?),
        VariantCommand::Show { name, app } => {
            print(&api.get::<Value>(&format!("{}/{}", variants(&app), segment(&name)))?)
        }
        VariantCommand::Logs {
            name,
            app,
            version,
            limit,
        } => {
            let mut path = format!("{}/{}/logs?limit={limit}", variants(&app), segment(&name));
            if let Some(version) = version {
                path.push_str(&format!("&version={version}"));
            }
            print(&api.get::<Value>(&path)?)
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

fn create(
    api: &Api,
    app: &str,
    name: &str,
    path: &Path,
    command: Option<String>,
    commit: Option<String>,
    sign_key: &str,
) -> anyhow::Result<()> {
    let key = signing::private_key(sign_key)?;
    if let Some(command) = &command
        && !command.contains("{port}")
    {
        bail!("--command must contain {{port}}, where the variant must listen");
    }
    let commit = commit.or_else(|| git_head(path));
    let packed = bundle::pack(path)?;
    let upload: Upload = api.send(
        Method::POST,
        &format!("{}/bundles", app::path(app)),
        &json!({ "sha256": packed.sha256, "size": packed.size }),
    )?;
    if !upload.exists {
        let url = upload
            .upload_url
            .context("the server returned no upload URL")?;
        let mut request = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3600))
            .build()?
            .put(url)
            .body(reqwest::blocking::Body::sized(
                File::open(packed.file.path())?,
                packed.size,
            ));
        for (header, value) in &upload.upload_headers {
            if !header.eq_ignore_ascii_case("content-length") {
                request = request.header(header, value);
            }
        }
        let response = request.send().context("could not upload the bundle")?;
        if !response.status().is_success() {
            bail!(
                "the bundle upload failed with {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            );
        }
    }
    let message = signing::message(
        app,
        name,
        &packed.sha256,
        command.as_deref(),
        commit.as_deref(),
    );
    let (signature, public_key) = signing::sign(&key, &message);
    let body = json!({
        "name": name,
        "sha256": packed.sha256,
        "command": command,
        "commit": commit,
        "signature": signature,
        "public_key": public_key,
    });
    print(&api.send::<Value>(Method::POST, &variants(app), &body)?)
}

/// The Git commit checked out at `path`, if it is in a repository.
fn git_head(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !commit.is_empty()).then_some(commit)
}
