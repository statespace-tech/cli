//! A small blocking client for the Statespace HTTP API.

use std::time::Duration;

use anyhow::{Context, bail};
use reqwest::{
    Method, StatusCode,
    blocking::{Client, RequestBuilder, Response},
};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub struct Api {
    client: Client,
    endpoint: String,
    token: Option<String>,
}

impl Api {
    pub fn new(endpoint: &str, token: Option<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("statespace-cli/", env!("CARGO_PKG_VERSION")))
                .build()?,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            token,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn with_token(&self, token: String) -> Self {
        Self {
            client: self.client.clone(),
            endpoint: self.endpoint.clone(),
            token: Some(token),
        }
    }

    /// A request without credentials, for sign-in.
    pub fn anonymous(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.endpoint))
    }

    pub fn request(&self, method: Method, path: &str) -> anyhow::Result<RequestBuilder> {
        let token = self
            .token
            .as_deref()
            .context("not signed in; run `ssp login` or set SSP_API_KEY")?;
        Ok(self.anonymous(method, path).bearer_auth(token))
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        decode(self.request(Method::GET, path)?.send()?)
    }

    pub fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &Value,
    ) -> anyhow::Result<T> {
        decode(self.request(method, path)?.json(body).send()?)
    }

    pub fn delete(&self, path: &str) -> anyhow::Result<()> {
        check(self.request(Method::DELETE, path)?.send()?).map(drop)
    }
}

/// Decode a successful JSON response or turn an error response into a message.
pub fn decode<T: DeserializeOwned>(response: Response) -> anyhow::Result<T> {
    let bytes = check(response)?.bytes()?;
    serde_json::from_slice(&bytes).context("the server returned invalid JSON")
}

fn check(response: Response) -> anyhow::Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == StatusCode::UNAUTHORIZED {
        bail!("the credential is invalid or expired; run `ssp login` or set SSP_API_KEY");
    }
    let body = response.text().unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| Some(value.get("error")?.as_str()?.to_owned()))
        .unwrap_or(body);
    bail!("{status}: {message}")
}

/// Percent-encode one URL path segment.
pub fn segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::segment;

    #[test]
    fn encodes_path_segments() {
        assert_eq!(segment("rank-v2.1"), "rank-v2.1");
        assert_eq!(segment("a/b?c"), "a%2Fb%3Fc");
    }
}
