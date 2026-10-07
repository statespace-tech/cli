//! Keys, sessions, database credentials, and audit records.

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use super::print;
use crate::{
    api::{Api, segment},
    cli::{CredentialCommand, DatabaseCommand, KeyCommand, TokenCommand},
};

pub fn key(api: &Api, command: KeyCommand) -> anyhow::Result<()> {
    match command {
        KeyCommand::Create {
            name,
            preset,
            scopes,
            expires_in_days,
        } => {
            let scopes = match preset {
                Some(preset) => preset.scopes(),
                None => scopes.iter().map(String::as_str).collect(),
            };
            let body =
                json!({ "name": name, "scopes": scopes, "expires_in_days": expires_in_days });
            print(&api.send::<Value>(Method::POST, "/v1/keys", &body)?)
        }
        KeyCommand::List => print(&api.get::<Value>("/v1/keys")?),
        KeyCommand::Revoke { id } => {
            api.delete(&format!("/v1/keys/{}", segment(&id)))?;
            print(&json!({ "revoked": id }))
        }
    }
}

pub fn token(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command {
        TokenCommand::List => print(&api.get::<Value>("/v1/tokens")?),
        TokenCommand::Revoke { id } => {
            api.delete(&format!("/v1/tokens/{}", segment(&id)))?;
            print(&json!({ "revoked": id }))
        }
        TokenCommand::RevokeAll { yes } => {
            if !yes {
                bail!("pass --yes to revoke every key and session for this account");
            }
            print(&api.send::<Value>(Method::POST, "/v1/credentials/revoke-all", &json!({}))?)
        }
    }
}

pub fn database(api: &Api, command: DatabaseCommand) -> anyhow::Result<()> {
    let DatabaseCommand::Credential(command) = command;
    match command {
        CredentialCommand::Create { name } => print(&api.send::<Value>(
            Method::POST,
            "/v1/database/credentials",
            &json!({ "name": name }),
        )?),
        CredentialCommand::List => print(&api.get::<Value>("/v1/database/credentials")?),
        CredentialCommand::Revoke { id } => {
            api.delete(&format!("/v1/database/credentials/{}", segment(&id)))?;
            print(&json!({ "revoked": id }))
        }
    }
}

pub fn audit(api: &Api, limit: u32, before: Option<i64>) -> anyhow::Result<()> {
    let mut path = format!("/v1/audit?limit={limit}");
    if let Some(before) = before {
        path.push_str(&format!("&before={before}"));
    }
    print(&api.get::<Value>(&path)?)
}
