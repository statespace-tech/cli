//! The public listener: assign each request, then forward it to the
//! process that serves its subject.

use std::{
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, RwLock,
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
    assign::{self, Decision, Key, Routing, SUBJECT_COOKIE},
    processes::Supervisor,
    telemetry::{self, Telemetry},
};

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// The routing a host has, and when it last heard from Statespace.
pub struct Current {
    pub routing: Routing,
    pub fetched: Option<Instant>,
}

pub struct Shared {
    pub current: RwLock<Current>,
    pub supervisor: Supervisor,
    pub telemetry: Arc<Telemetry>,
    pub local: bool,
    pub capture_http: bool,
    pub stale_after: Duration,
    pub in_flight: AtomicUsize,
    client: Client<HttpConnector, ProxyBody>,
}

impl Shared {
    pub fn new(
        supervisor: Supervisor,
        telemetry: Arc<Telemetry>,
        local: bool,
        capture_http: bool,
        stale_after: Duration,
    ) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        Self {
            current: RwLock::new(Current {
                routing: Routing::default(),
                fetched: None,
            }),
            supervisor,
            telemetry,
            local,
            capture_http,
            stale_after,
            in_flight: AtomicUsize::new(0),
            client: Client::builder(TokioExecutor::new())
                .pool_idle_timeout(Duration::from_secs(30))
                .build(connector),
        }
    }

    /// Decide who serves a request. A host without a recent configuration
    /// serves every subject with control.
    fn decide(&self, request: &serde_json::Value, headers: &HeaderMap) -> Decision {
        if self.local {
            let chosen = headers
                .get("x-statespace-variant")
                .and_then(|value| value.to_str().ok())
                .map(|name| Key {
                    name: name.to_owned(),
                    version: None,
                });
            return Decision {
                serve: chosen.unwrap_or_else(Key::control),
                assignment: None,
            };
        }
        let current = self.current.read().expect("routing is never poisoned");
        let fresh = current
            .fetched
            .is_some_and(|fetched| fetched.elapsed() <= self.stale_after);
        if !fresh {
            return Decision {
                serve: Key::control(),
                assignment: None,
            };
        }
        assign::decide(&current.routing, request, |key| {
            self.supervisor.port(key).is_some()
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
    let decision = shared.decide(&view, request.headers());
    if let Some(assignment) = &decision.assignment {
        shared.telemetry.run(assignment, method.as_str(), &path);
    }
    let mut serve = decision.serve.clone();
    let Some(port) = shared.supervisor.port(&serve).or_else(|| {
        serve = Key::control();
        shared.supervisor.port(&serve)
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
    parts.uri = upstream_uri(port, &parts.uri);
    // Clients may speak HTTP/2; processes are reached over HTTP/1.1.
    parts.version = http::Version::HTTP_11;
    let upstream = Request::from_parts(parts, body.boxed());

    let mut response = match shared.client.request(upstream).await {
        Ok(response) => response,
        Err(error) => {
            shared.supervisor.suspect(&serve);
            let fallback = retry
                .filter(|_| !serve.is_control())
                .and_then(|retry| Some((retry, shared.supervisor.port(&Key::control())?)));
            let Some(((method, uri, headers), control_port)) = fallback else {
                eprintln!("[ssp] {} did not answer: {error}", serve.name);
                return text(StatusCode::BAD_GATEWAY, "the app did not answer");
            };
            serve = Key::control();
            let mut again = Request::new(Empty::new().map_err(|never| match never {}).boxed());
            *again.method_mut() = method;
            *again.uri_mut() = upstream_uri(control_port, &uri);
            *again.headers_mut() = headers;
            again
                .headers_mut()
                .insert("x-statespace-variant", HeaderValue::from_static("control"));
            match shared.client.request(again).await {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("[ssp] control did not answer: {error}");
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
        shared
            .telemetry
            .record_http(&serve, status.as_u16(), first_byte_ms);
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
    let capture = shared.capture_http;
    let assignment = decision.assignment;
    let request_bytes = view["headers"]["content-length"]
        .as_str()
        .and_then(|length| length.parse::<i64>().ok());
    let finish = Box::new(move |response_bytes: u64| {
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        telemetry.record_http(&serve, status.as_u16(), duration_ms);
        if capture {
            telemetry.request(json!({
                "subject_id": subject,
                "experiment": assignment.as_ref().map(|assignment| &assignment.experiment),
                "experiment_version": assignment.as_ref().map(|assignment| assignment.version),
                "variant": serve.name,
                "variant_version": serve.version,
                "method": method.as_str(),
                "path": path,
                "status": status.as_u16(),
                "duration_ms": duration_ms,
                "first_byte_ms": first_byte_ms,
                "request_bytes": request_bytes,
                "response_bytes": response_bytes,
            }));
        }
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

/// Point the request at a process and tell it who the subject is. Clients
/// cannot set Statespace headers themselves.
fn prepare_headers(
    request: &mut Request<Incoming>,
    peer: SocketAddr,
    serve: &Key,
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
    set("x-statespace-variant", &serve.name);
    if let Some(subject) = subject {
        set("x-statespace-subject", subject);
    }
    if let Some(assignment) = &decision.assignment {
        set("x-statespace-experiment", &assignment.experiment);
    }
}

/// Requests that can be sent again to control when a variant fails: those
/// without a body.
fn replayable(request: &Request<Incoming>) -> bool {
    matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::DELETE
    ) && request.body().is_end_stream()
}

fn upstream_uri(port: u16, uri: &http::Uri) -> http::Uri {
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    format!("http://127.0.0.1:{port}{path}")
        .parse()
        .expect("a port and a request path form a URI")
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
    fn targets_the_process_port() {
        let uri: http::Uri = "https://example.com/a/b?c=d".parse().unwrap();
        assert_eq!(
            upstream_uri(9001, &uri).to_string(),
            "http://127.0.0.1:9001/a/b?c=d"
        );
    }
}
