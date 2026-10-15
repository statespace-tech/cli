//! The public listener: assign each request, then forward it to the
//! default or to the arm that serves its subject.

use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use serde_json::json;

use super::{
    assign::{self, Decision, Key, Routing, SUBJECT_COOKIE, Target},
    docker::Supervisor,
    telemetry::{self, Telemetry},
};

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// How long a remote arm that failed a request gets no traffic.
const SUSPECT_FOR: Duration = Duration::from_secs(10);

/// The routing a proxy has, and when it last heard from Statespace.
pub struct Current {
    pub routing: Routing,
    pub fetched: Option<Instant>,
}

/// Where the default and the arms take traffic.
pub enum Upstreams {
    /// Containers this proxy runs.
    Docker(Supervisor),
    /// The default beside the proxy, and arms a controller runs elsewhere.
    Kubernetes {
        default: String,
        arms: RwLock<HashMap<Key, String>>,
        suspects: Mutex<HashMap<Key, Instant>>,
    },
}

impl Upstreams {
    pub fn address(&self, target: &Target) -> Option<String> {
        match (self, target) {
            (Self::Docker(supervisor), target) => supervisor.address(target),
            (Self::Kubernetes { default, .. }, Target::Default) => Some(default.clone()),
            (Self::Kubernetes { arms, suspects, .. }, Target::Arm(key)) => {
                let suspected = suspects
                    .lock()
                    .expect("suspects are never poisoned")
                    .get(key)
                    .is_some_and(|since| since.elapsed() < SUSPECT_FOR);
                if suspected {
                    return None;
                }
                arms.read()
                    .expect("arms are never poisoned")
                    .get(key)
                    .cloned()
            }
        }
    }

    pub fn suspect(&self, target: &Target) {
        match (self, target) {
            (Self::Docker(supervisor), target) => supervisor.suspect(target),
            (Self::Kubernetes { suspects, .. }, Target::Arm(key)) => {
                suspects
                    .lock()
                    .expect("suspects are never poisoned")
                    .insert(key.clone(), Instant::now());
            }
            (Self::Kubernetes { .. }, Target::Default) => {}
        }
    }
}

pub struct Shared {
    pub current: RwLock<Current>,
    pub upstreams: Upstreams,
    pub telemetry: Arc<Telemetry>,
    pub stale_after: Duration,
    pub in_flight: AtomicUsize,
    client: Client<HttpConnector, ProxyBody>,
}

impl Shared {
    pub fn new(upstreams: Upstreams, telemetry: Arc<Telemetry>, stale_after: Duration) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        Self {
            current: RwLock::new(Current {
                routing: Routing::default(),
                fetched: None,
            }),
            upstreams,
            telemetry,
            stale_after,
            in_flight: AtomicUsize::new(0),
            client: Client::builder(TokioExecutor::new())
                .pool_idle_timeout(Duration::from_secs(30))
                .build(connector),
        }
    }

    /// Decide who serves a request. A proxy without a recent configuration
    /// sends every subject to the default.
    fn decide(&self, request: &serde_json::Value) -> Decision {
        let current = self.current.read().expect("routing is never poisoned");
        let fresh = current
            .fetched
            .is_some_and(|fetched| fetched.elapsed() <= self.stale_after);
        if !fresh {
            return Decision {
                serve: Target::Default,
                assignment: None,
            };
        }
        assign::decide(&current.routing, request, |key| {
            self.upstreams.address(&Target::Arm(key.clone())).is_some()
        })
    }
}

/// Accept connections until `shutdown` turns true.
pub async fn serve(
    listener: tokio::net::TcpListener,
    shared: Arc<Shared>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    eprintln!("[ssp] could not accept a connection: {error}");
                    continue;
                }
            },
            _ = shutdown.wait_for(|stop| *stop) => return,
        };
        let _ = stream.set_nodelay(true);
        let shared = shared.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let shared = shared.clone();
                async move { Ok::<_, Infallible>(handle(shared, peer, request).await) }
            });
            let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        });
    }
}

async fn handle(
    shared: Arc<Shared>,
    peer: SocketAddr,
    request: Request<Incoming>,
) -> Response<ProxyBody> {
    shared.in_flight.fetch_add(1, Ordering::SeqCst);
    let response = forward(&shared, peer, request).await;
    shared.in_flight.fetch_sub(1, Ordering::SeqCst);
    response
}

async fn forward(
    shared: &Arc<Shared>,
    peer: SocketAddr,
    mut request: Request<Incoming>,
) -> Response<ProxyBody> {
    let started = Instant::now();
    let mut cookies = assign::cookies(request.headers());
    let new_subject = (!cookies.contains_key(SUBJECT_COOKIE)).then(|| telemetry::id("sid"));
    if let Some(subject) = &new_subject {
        cookies.insert(SUBJECT_COOKIE.into(), json!(subject));
    }
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let view = assign::request_value(
        method.as_str(),
        &path,
        request.uri().query(),
        request.headers(),
        &cookies,
    );
    let decision = shared.decide(&view);
    if let Some(assignment) = &decision.assignment {
        shared.telemetry.run(assignment, method.as_str(), &path);
    }
    let mut serve = decision.serve.clone();
    let Some(address) = shared.upstreams.address(&serve).or_else(|| {
        serve = Target::Default;
        shared.upstreams.address(&serve)
    }) else {
        return text(StatusCode::SERVICE_UNAVAILABLE, "the app is starting");
    };

    let subject = decision
        .assignment
        .as_ref()
        .map(|assignment| assignment.subject.clone())
        .or_else(|| {
            cookies
                .get(SUBJECT_COOKIE)
                .and_then(|value| value.as_str().map(Into::into))
        });
    prepare_headers(&mut request, peer, &serve, subject.as_deref(), &decision);
    let upgrade = is_upgrade(request.headers());
    let retry = (!upgrade && replayable(&request)).then(|| {
        (
            request.method().clone(),
            request.uri().clone(),
            request.headers().clone(),
        )
    });
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut request));
    let (mut parts, body) = request.into_parts();
    parts.uri = upstream_uri(&address, &parts.uri);
    // Clients may speak HTTP/2; the app is reached over HTTP/1.1.
    parts.version = http::Version::HTTP_11;
    let upstream = Request::from_parts(parts, body.boxed());

    let mut response = match shared.client.request(upstream).await {
        Ok(response) => response,
        Err(error) => {
            shared.upstreams.suspect(&serve);
            let fallback = retry
                .filter(|_| serve != Target::Default)
                .and_then(|retry| Some((retry, shared.upstreams.address(&Target::Default)?)));
            let Some(((method, uri, headers), default)) = fallback else {
                eprintln!("[ssp] {serve} did not answer: {error}");
                return text(StatusCode::BAD_GATEWAY, "the app did not answer");
            };
            serve = Target::Default;
            let mut again = Request::new(Empty::new().map_err(|never| match never {}).boxed());
            *again.method_mut() = method;
            *again.uri_mut() = upstream_uri(&default, &uri);
            *again.headers_mut() = headers;
            again
                .headers_mut()
                .insert("x-statespace-variant", HeaderValue::from_static("default"));
            match shared.client.request(again).await {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("[ssp] the default did not answer: {error}");
                    return text(StatusCode::BAD_GATEWAY, "the app did not answer");
                }
            }
        }
    };
    let status = response.status();
    let first_byte_ms = started.elapsed().as_secs_f64() * 1000.0;

    if status == StatusCode::SWITCHING_PROTOCOLS {
        if let Some(client_upgrade) = client_upgrade {
            let upstream_upgrade = hyper::upgrade::on(&mut response);
            tokio::spawn(async move {
                if let (Ok(client), Ok(upstream)) = (client_upgrade.await, upstream_upgrade.await) {
                    let _ = tokio::io::copy_bidirectional(
                        &mut TokioIo::new(client),
                        &mut TokioIo::new(upstream),
                    )
                    .await;
                }
            });
        }
        if let Target::Arm(key) = &serve {
            shared
                .telemetry
                .record_http(key, status.as_u16(), first_byte_ms);
        }
        let (parts, _) = response.into_parts();
        return Response::from_parts(parts, Empty::new().map_err(|never| match never {}).boxed());
    }

    let (mut parts, body) = response.into_parts();
    remove_hop_by_hop(&mut parts.headers);
    if let Some(subject) = &new_subject {
        let secure = view["headers"]["x-forwarded-proto"] == "https";
        let cookie = format!(
            "{SUBJECT_COOKIE}={subject}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax{}",
            if secure { "; Secure" } else { "" }
        );
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            parts.headers.append(header::SET_COOKIE, value);
        }
    }
    let telemetry = shared.telemetry.clone();
    let request_bytes = view["headers"]["content-length"]
        .as_str()
        .and_then(|length| length.parse::<i64>().ok());
    // Only arms are recorded; the default is the app as it always runs.
    let finish = Box::new(move |response_bytes: u64| {
        let Target::Arm(key) = serve else {
            return;
        };
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        telemetry.record_http(&key, status.as_u16(), duration_ms);
        telemetry.request(json!({
            "subject_id": subject,
            "experiment": key.experiment,
            "experiment_version": key.experiment_version,
            "variant": key.arm,
            "variant_version": key.version,
            "method": method.as_str(),
            "path": path,
            "status": status.as_u16(),
            "duration_ms": duration_ms,
            "first_byte_ms": first_byte_ms,
            "request_bytes": request_bytes,
            "response_bytes": response_bytes,
        }));
    });
    Response::from_parts(
        parts,
        Metered {
            inner: body,
            bytes: 0,
            finish: Some(finish),
        }
        .boxed(),
    )
}

/// Tell the app who the subject is and what serves it. Clients cannot set
/// Statespace headers themselves.
fn prepare_headers(
    request: &mut Request<Incoming>,
    peer: SocketAddr,
    serve: &Target,
    subject: Option<&str>,
    decision: &Decision,
) {
    let authority = request
        .uri()
        .authority()
        .map(|authority| authority.to_string());
    let upgrade = is_upgrade(request.headers());
    let headers = request.headers_mut();
    if !upgrade {
        remove_hop_by_hop(headers);
    }
    let reserved: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-statespace-"))
        .cloned()
        .collect();
    for name in reserved {
        headers.remove(name);
    }
    if !headers.contains_key(header::HOST)
        && let Some(value) = authority.and_then(|authority| HeaderValue::from_str(&authority).ok())
    {
        headers.insert(header::HOST, value);
    }
    let forwarded = match headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        Some(previous) => format!("{previous}, {}", peer.ip()),
        None => peer.ip().to_string(),
    };
    let mut set = |name: &'static str, value: &str| {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    };
    set("x-forwarded-for", &forwarded);
    set(
        "x-statespace-variant",
        match serve {
            Target::Default => "default",
            Target::Arm(key) => &key.arm,
        },
    );
    if let Some(subject) = subject {
        set("x-statespace-subject", subject);
    }
    if let Some(assignment) = &decision.assignment {
        set("x-statespace-experiment", &assignment.experiment);
    }
}

/// Requests that can be sent again to the default when an arm fails: those
/// without a body.
fn replayable(request: &Request<Incoming>) -> bool {
    matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::DELETE
    ) && request.body().is_end_stream()
}

/// The request's path and query at `address`, such as `http://127.0.0.1:41022`.
fn upstream_uri(address: &str, uri: &http::Uri) -> http::Uri {
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    format!("{}{path}", address.trim_end_matches('/'))
        .parse()
        .unwrap_or_else(|_| http::Uri::from_static("http://127.0.0.1:9/"))
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(header::UPGRADE)
        && headers
            .get_all(header::CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.to_ascii_lowercase().contains("upgrade"))
}

/// Remove headers that describe one connection rather than the message.
fn remove_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    for name in named {
        headers.remove(name.as_str());
    }
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
}

fn text(status: StatusCode, message: &'static str) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(Bytes::from_static(message.as_bytes()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response
}

/// A response body that reports its size once it ends or is dropped, so
/// streamed responses are timed to their last byte.
struct Metered {
    inner: Incoming,
    bytes: u64,
    finish: Option<Box<dyn FnOnce(u64) + Send + Sync>>,
}

impl Metered {
    fn done(&mut self) {
        if let Some(finish) = self.finish.take() {
            finish(self.bytes);
        }
    }
}

impl Body for Metered {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_frame(context);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.bytes += data.len() as u64;
                }
            }
            Poll::Ready(None) | Poll::Ready(Some(Err(_))) => this.done(),
            Poll::Pending => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for Metered {
    fn drop(&mut self) {
        self.done();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_connection_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "keep-alive, x-private".parse().unwrap());
        headers.insert("x-private", "1".parse().unwrap());
        headers.insert("keep-alive", "timeout=5".parse().unwrap());
        headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
        remove_hop_by_hop(&mut headers);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains_key(header::CONTENT_TYPE));
    }

    #[test]
    fn recognizes_upgrades() {
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, "websocket".parse().unwrap());
        assert!(!is_upgrade(&headers));
        headers.insert(header::CONNECTION, "keep-alive, Upgrade".parse().unwrap());
        assert!(is_upgrade(&headers));
    }

    #[test]
    fn targets_the_upstream_address() {
        let uri: http::Uri = "https://example.com/a/b?c=d".parse().unwrap();
        assert_eq!(
            upstream_uri("http://127.0.0.1:9001", &uri).to_string(),
            "http://127.0.0.1:9001/a/b?c=d"
        );
        assert_eq!(
            upstream_uri("http://shop-price-control.shop.svc:8000/", &uri).to_string(),
            "http://shop-price-control.shop.svc:8000/a/b?c=d"
        );
    }

    #[test]
    fn suspected_remote_arms_get_no_traffic_for_a_while() {
        let key = Key {
            experiment: "price".into(),
            experiment_version: 1,
            arm: "control".into(),
            version: None,
        };
        let upstreams = Upstreams::Kubernetes {
            default: "http://localhost:8000".into(),
            arms: RwLock::new(HashMap::from([(key.clone(), "http://arm".to_owned())])),
            suspects: Mutex::default(),
        };
        let arm = Target::Arm(key);
        assert_eq!(upstreams.address(&arm).as_deref(), Some("http://arm"));
        upstreams.suspect(&arm);
        assert_eq!(upstreams.address(&arm), None);
        assert_eq!(
            upstreams.address(&Target::Default).as_deref(),
            Some("http://localhost:8000")
        );
    }
}
