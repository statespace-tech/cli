//! The connection of `ssp proxy` and `ssp controller` to Statespace:
//! configuration, control pins, reports, and events.

use std::time::Duration;

use anyhow::bail;
use serde::Deserialize;
use serde_json::{Value, json};

use super::assign::HostConfig;
use crate::api::segment;

#[derive(Clone)]
pub struct Remote {
    client: reqwest::Client,
    endpoint: String,
    token: String,
    app: String,
}

/// The default image a version's control arm runs.
#[derive(Deserialize)]
pub struct ControlPin {
    pub image: String,
}

impl Remote {
    pub fn new(endpoint: &str, token: String, app: String) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .user_agent(concat!("statespace-proxy/", env!("CARGO_PKG_VERSION")))
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

    async fn post(&self, url: String, body: &Value) -> anyhow::Result<reqwest::Response> {
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30))
            .json(body)
            .send()
            .await?;
        Self::check(response).await
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

    /// Report this proxy or controller: its kind, version, and capacity.
    pub async fn report(
        &self,
        id: &str,
        kind: &str,
        max_variants: u32,
        report: Value,
    ) -> anyhow::Result<()> {
        let body = json!({
            "id": id,
            "kind": kind,
            "version": env!("CARGO_PKG_VERSION"),
            "max_variants": max_variants,
            "report": report,
        });
        self.post(self.url("/hosts"), &body).await.map(drop)
    }

    /// Propose `image` as the control arm of a running version. Returns the
    /// pin every proxy and controller uses, which may be another's.
    pub async fn pin_control(
        &self,
        experiment: &str,
        version: u32,
        image: &str,
    ) -> anyhow::Result<ControlPin> {
        let url = self.url(&format!(
            "/experiments/{}/versions/{version}/control",
            segment(experiment)
        ));
        Ok(self
            .post(url, &json!({ "image": image }))
            .await?
            .json()
            .await?)
    }

    /// Report every arm a controller runs.
    pub async fn report_arms(&self, controller: &str, arms: Vec<Value>) -> anyhow::Result<()> {
        let body = json!({ "controller": controller, "arms": arms });
        self.post(self.url("/arms"), &body).await.map(drop)
    }

    /// Send events of this app, as `{table: [events]}`.
    pub async fn send_events(&self, mut batch: Value) -> anyhow::Result<()> {
        batch["app"] = json!(self.app);
        self.post(format!("{}/v1/events", self.endpoint), &batch)
            .await
            .map(drop)
    }
}
