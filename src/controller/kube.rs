//! A small client for the Kubernetes API, as a pod's service account sees
//! it. Outside a cluster, `STATESPACE_KUBE_API` names an API that needs no
//! token, such as the one `kubectl proxy` serves.

use std::time::Duration;

use anyhow::{Context, bail};
use reqwest::{Method, StatusCode};
use serde_json::Value;

const SERVICE_ACCOUNT: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

#[derive(Clone)]
pub struct Kube {
    client: reqwest::Client,
    base: String,
    /// Where the service account's token is; it is read for each request
    /// because Kubernetes rotates it.
    token: Option<String>,
    pub namespace: String,
}

impl Kube {
    pub fn connect(namespace: Option<String>) -> anyhow::Result<Self> {
        if let Ok(base) = std::env::var("STATESPACE_KUBE_API") {
            return Ok(Self {
                client: reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(10))
                    .build()?,
                base: base.trim_end_matches('/').to_owned(),
                token: None,
                namespace: namespace.unwrap_or_else(|| "default".into()),
            });
        }
        let host = std::env::var("KUBERNETES_SERVICE_HOST").context(
            "ssp controller runs in a Kubernetes pod; KUBERNETES_SERVICE_HOST is not set",
        )?;
        let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
        let certificate = std::fs::read(format!("{SERVICE_ACCOUNT}/ca.crt"))
            .context("could not read the service account's CA certificate")?;
        let namespace = match namespace {
            Some(namespace) => namespace,
            None => std::fs::read_to_string(format!("{SERVICE_ACCOUNT}/namespace"))
                .context("could not read the pod's namespace; pass --namespace")?
                .trim()
                .to_owned(),
        };
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .add_root_certificate(reqwest::Certificate::from_pem(&certificate)?)
                .build()?,
            base: format!("https://{host}:{port}"),
            token: Some(format!("{SERVICE_ACCOUNT}/token")),
            namespace,
        })
    }

    /// `apis/apps/v1/namespaces/<namespace>/<resource>`, and the like.
    pub fn path(&self, group: &str, resource: &str) -> String {
        let prefix = if group == "v1" {
            "api/v1".to_owned()
        } else {
            format!("apis/{group}")
        };
        format!("/{prefix}/namespaces/{}/{resource}", self.namespace)
    }

    fn request(&self, method: Method, path: &str) -> anyhow::Result<reqwest::RequestBuilder> {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            let token = std::fs::read_to_string(token)
                .context("could not read the service account's token")?;
            request = request.bearer_auth(token.trim());
        }
        Ok(request)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Option<Value>> {
        let response = request.timeout(Duration::from_secs(30)).send().await?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|value| Some(value.get("message")?.as_str()?.to_owned()))
                .unwrap_or(body);
            bail!("Kubernetes answered {status}: {message}");
        }
        Ok(Some(serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    /// An object, or `None` when it does not exist.
    pub async fn get(&self, path: &str) -> anyhow::Result<Option<Value>> {
        self.send(self.request(Method::GET, path)?).await
    }

    /// The items of a list, filtered by a label selector.
    pub async fn list(&self, path: &str, selector: &str) -> anyhow::Result<Vec<Value>> {
        let request = self
            .request(Method::GET, path)?
            .query(&[("labelSelector", selector)]);
        Ok(self
            .send(request)
            .await?
            .and_then(|list| list["items"].as_array().cloned())
            .unwrap_or_default())
    }

    pub async fn create(&self, path: &str, object: &Value) -> anyhow::Result<Value> {
        self.send(self.request(Method::POST, path)?.json(object))
            .await?
            .with_context(|| format!("{path} does not exist"))
    }

    pub async fn replace(&self, path: &str, object: &Value) -> anyhow::Result<Value> {
        self.send(self.request(Method::PUT, path)?.json(object))
            .await?
            .with_context(|| format!("{path} does not exist"))
    }

    pub async fn delete(&self, path: &str) -> anyhow::Result<()> {
        let request = self
            .request(Method::DELETE, path)?
            .json(&serde_json::json!({"propagationPolicy": "Background"}));
        self.send(request).await.map(drop)
    }

    /// Follow a container's log from `since`, with a timestamp on each line.
    pub async fn follow(
        &self,
        pod: &str,
        container: &str,
        since: Option<&str>,
    ) -> anyhow::Result<reqwest::Response> {
        let mut query = vec![
            ("container", container.to_owned()),
            ("follow", "true".to_owned()),
            ("timestamps", "true".to_owned()),
        ];
        if let Some(since) = since {
            query.push(("sinceTime", since.to_owned()));
        }
        let response = self
            .request(
                Method::GET,
                &format!("{}/{pod}/log", self.path("v1", "pods")),
            )?
            .query(&query)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!(
                "Kubernetes answered {} for the log of {pod}",
                response.status()
            );
        }
        Ok(response)
    }
}
