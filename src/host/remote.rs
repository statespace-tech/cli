//! The host's connection to Statespace: configuration, bundles, reports,
//! and events.

use std::{path::Path, time::Duration};

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::assign::HostConfig;
use crate::api::segment;

#[derive(Clone)]
pub struct Remote {
    client: reqwest::Client,
    endpoint: String,
    token: String,
    app: String,
}

#[derive(Deserialize)]
struct Download {
    download_url: String,
}

impl Remote {
    pub fn new(endpoint: &str, token: String, app: String) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .user_agent(concat!("statespace-host/", env!("CARGO_PKG_VERSION")))
                .build()?,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            token,
            app,
        })
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/v1/runtime/apps/{}{path}",
            self.endpoint,
            segment(&self.app)
        )
    }

    async fn check(response: reqwest::Response) -> anyhow::Result<reqwest::Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| Some(value.get("error")?.as_str()?.to_owned()))
            .unwrap_or(body);
        bail!("{status}: {message}")
    }

    pub async fn config(&self) -> anyhow::Result<HostConfig> {
        let response = self
            .client
            .get(self.url(""))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .send()
            .await?;
        Ok(Self::check(response).await?.json().await?)
    }

    pub async fn report(&self, id: &str, max_variants: u32, report: Value) -> anyhow::Result<()> {
        let response = self
            .client
            .post(self.url("/hosts"))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .json(&json!({ "id": id, "max_variants": max_variants, "report": report }))
            .send()
            .await?;
        Self::check(response).await.map(drop)
    }

    /// Send events of this app, as `{table: [events]}`.
    pub async fn send_events(&self, mut batch: Value) -> anyhow::Result<()> {
        batch["app"] = json!(self.app);
        let response = self
            .client
            .post(format!("{}/v1/events", self.endpoint))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30))
            .json(&batch)
            .send()
            .await?;
        Self::check(response).await.map(drop)
    }

    /// Download a bundle to `path`, checking its SHA-256 and size.
    pub async fn download(&self, sha256: &str, size: i64, path: &Path) -> anyhow::Result<()> {
        let response = self
            .client
            .get(self.url(&format!("/bundles/{}", segment(sha256))))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .send()
            .await?;
        let download: Download = Self::check(response).await?.json().await?;
        let mut response = Self::check(
            self.client
                .get(download.download_url)
                .timeout(Duration::from_secs(3600))
                .send()
                .await?,
        )
        .await
        .context("could not download the bundle")?;
        let mut file = tokio::fs::File::create(path).await?;
        let mut hasher = Sha256::new();
        let mut received = 0_i64;
        while let Some(chunk) = response.chunk().await? {
            received += chunk.len() as i64;
            if received > size {
                bail!("the bundle is larger than its recorded size");
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        let actual = format!("{:x}", hasher.finalize());
        if actual != sha256 || received != size {
            bail!("the downloaded bundle does not match its SHA-256");
        }
        Ok(())
    }
}
