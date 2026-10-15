//! Checks of the `ssp` binary. The Docker test also needs Docker; it runs
//! when `STATESPACE_DOCKER_TESTS=1`.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use assert_cmd::{Command, cargo::cargo_bin};
use serde_json::{Value, json};

/// Run `ssp` with an empty configuration so no saved session is used.
fn ssp(arguments: &[&str]) -> (bool, Value, Value) {
    let config = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("ssp")
        .unwrap()
        .env("STATESPACE_CONFIG", config.path().join("config.toml"))
        .env_remove("SSP_TOKEN")
        .env_remove("SSP_SIGNING_KEY")
        .env_remove("SSP_TRUST_KEY")
        .env_remove("STATESPACE_APP")
        .env("STATESPACE_URL", "http://127.0.0.1:9")
        .args(arguments)
        .output()
        .unwrap();
    let parse = |bytes: &[u8]| serde_json::from_slice(bytes).unwrap_or(Value::Null);
    (
        output.status.success(),
        parse(&output.stdout),
        parse(&output.stderr),
    )
}

fn error(arguments: &[&str]) -> String {
    let (ok, _, error) = ssp(arguments);
    assert!(!ok, "{arguments:?} succeeded");
    error["error"].as_str().unwrap_or_default().to_owned()
}

#[test]
fn reports_errors_as_json() {
    assert!(error(&["experiment", "list"]).contains("ssp login"));
    assert!(error(&["experiment", "delete", "checkout"]).contains("--yes"));
    assert!(error(&["app", "delete", "web"]).contains("--yes"));
    assert!(error(&["token", "revoke", "--all"]).contains("--yes"));
    let message = error(&["experiment", "create", "checkout", "--app", "web"]);
    assert!(message.contains("--split"), "{message}");
    let message = error(&[
        "variant",
        "create",
        "fast",
        "--app",
        "web",
        "--image",
        "ghcr.io/acme/web:main",
        "--signing-key=not-a-key",
    ]);
    assert!(message.contains("signing key"), "{message}");
}

#[test]
fn prints_signing_key_pairs() {
    let (ok, keys, _) = ssp(&["keygen"]);
    assert!(ok);
    assert_eq!(keys["private_key"].as_str().unwrap().len(), 44);
    assert_eq!(keys["public_key"].as_str().unwrap().len(), 44);
}

#[test]
fn proxies_need_what_their_runtime_needs() {
    let base = ["proxy", "--app", "web", "--token", "t"];
    let docker = [&base[..], &["--runtime", "docker", "--image", "app:1"]].concat();
    assert!(error(&docker).contains("--trust-key"));
    let unsigned = [&base[..], &["--runtime", "docker", "--trust-key", "k"]].concat();
    assert!(error(&unsigned).contains("--image"));
    let kubernetes = [&base[..], &["--runtime", "kubernetes"]].concat();
    assert!(error(&kubernetes).contains("--default"));
    let mixed = [
        &base[..],
        &[
            "--runtime",
            "kubernetes",
            "--default",
            "http://localhost:8000",
            "--image",
            "app:1",
        ],
    ]
    .concat();
    assert!(error(&mixed).contains("Deployment"));
    let anonymous = ["proxy", "--app", "web", "--runtime", "kubernetes"];
    assert!(error(&anonymous).contains("SSP_TOKEN"));
}

/// Stops a child when a test ends, even when it fails.
struct Process(Child);

impl Process {
    /// Ask the process to stop, as a platform does, and wait for it to exit.
    fn stop(&mut self) -> Option<std::process::ExitStatus> {
        if let Ok(Some(status)) = self.0.try_wait() {
            return Some(status);
        }
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.0.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.stop().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// A stand-in for Statespace: it serves one app's configuration and keeps
/// what proxies and `ssp variant create` send.
#[derive(Default)]
struct Fake {
    config: Value,
    variants: Vec<Value>,
    pin: Option<String>,
    events: Vec<Value>,
    reports: Vec<Value>,
}

fn serve(fake: Arc<Mutex<Fake>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let fake = fake.clone();
            std::thread::spawn(move || answer(stream, &fake));
        }
    });
    port
}

fn answer(mut stream: TcpStream, fake: &Mutex<Fake>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut fake = fake.lock().unwrap();
    let (status, response) = match (method, path) {
        ("GET", path)
            if path.starts_with("/v1/runtime/apps/") && path.matches('/').count() == 4 =>
        {
            let mut config = fake.config.clone();
            if let (Some(pin), Some(experiment)) = (&fake.pin, config["experiments"].get_mut(0)) {
                experiment["control"]["image"] = json!(pin);
            }
            (200, config)
        }
        ("POST", path) if path.ends_with("/control") => {
            let pin = fake
                .pin
                .get_or_insert_with(|| body["image"].as_str().unwrap_or_default().to_owned())
                .clone();
            (200, json!({ "image": pin }))
        }
        ("POST", path) if path.ends_with("/hosts") => {
            fake.reports.push(body);
            (204, Value::Null)
        }
        ("POST", "/v1/events") => {
            fake.events.push(body);
            (202, json!({"accepted": 1}))
        }
        ("POST", path) if path.ends_with("/variants") => {
            fake.variants.push(body.clone());
            (201, body)
        }
        _ => (404, json!({"error": "not found"})),
    };
    let body = if response.is_null() {
        String::new()
    } else {
        response.to_string()
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn docker(arguments: &[&str]) -> String {
    let output = std::process::Command::new("docker")
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "docker {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Removes the test's registry and containers when the test ends.
struct Cleanup {
    registry: String,
    app: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .args(["rm", "--force", &self.registry])
            .output();
        let leftovers = std::process::Command::new("docker")
            .args([
                "ps",
                "--all",
                "--quiet",
                "--filter",
                &format!("label=statespace.com/app={}", self.app),
            ])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        for id in leftovers.split_whitespace() {
            let _ = std::process::Command::new("docker")
                .args(["rm", "--force", id])
                .output();
        }
    }
}

fn get(port: u16, subject: &str) -> Option<Value> {
    reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{port}/checkout?step=1"))
        .header("cookie", format!("ssp_sid={subject}"))
        .header("x-statespace-variant", "spoofed")
        .timeout(Duration::from_secs(5))
        .send()
        .ok()?
        .json()
        .ok()
}

/// A proxy on Docker runs the default and each arm as containers, signs
/// nothing it should not run, and records only the arms.
#[test]
fn docker_proxies_run_the_default_and_each_arm() {
    if std::env::var("STATESPACE_DOCKER_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set STATESPACE_DOCKER_TESTS=1 to run the Docker test");
        return;
    }
    let app = format!("ssptest{:06x}", rand_suffix());
    // The registry listens on the Docker host's own network, which the
    // daemon reaches as 127.0.0.1 also when it runs in a virtual machine.
    let registry_host = format!("127.0.0.1:{}", 20_000 + rand_suffix() % 20_000);
    let registry = docker(&[
        "run",
        "--detach",
        "--network",
        "host",
        "--env",
        &format!("REGISTRY_HTTP_ADDR={registry_host}"),
        "registry:2",
    ]);
    let _cleanup = Cleanup {
        registry: registry.clone(),
        app: app.clone(),
    };
    let repository = format!("{registry_host}/{app}");
    let push = |tag: &str, greeting: &str| {
        let reference = format!("{repository}:{tag}");
        docker(&[
            "build",
            "--quiet",
            "--build-arg",
            &format!("GREETING={greeting}"),
            "--tag",
            &reference,
            "examples/app",
        ]);
        let deadline = Instant::now() + Duration::from_secs(30);
        // The registry may still be starting.
        while std::process::Command::new("docker")
            .args(["push", "--quiet", &reference])
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true)
        {
            assert!(Instant::now() < deadline, "could not push {reference}");
            std::thread::sleep(Duration::from_millis(500));
        }
        reference
    };
    let default = push("default", "hello");
    let variant = push("lower-price", "hi");

    let fake = Arc::new(Mutex::new(Fake::default()));
    let api = format!("http://127.0.0.1:{}", serve(fake.clone()));
    let config = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let run = |arguments: &[&str]| {
        let output = std::process::Command::new(cargo_bin("ssp"))
            .env("STATESPACE_CONFIG", config.path().join("config.toml"))
            .env("STATESPACE_URL", &api)
            .env("SSP_TOKEN", "test")
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let keys = run(&["keygen"]);
    let private_key = keys["private_key"].as_str().unwrap();
    let public_key = keys["public_key"].as_str().unwrap();
    // A tag is published by its digest.
    let published = run(&[
        "variant",
        "create",
        "lower-price",
        "--app",
        &app,
        "--image",
        &variant,
        "--commit",
        "abc123",
        "--signing-key",
        private_key,
    ]);
    let image = published["image"].as_str().unwrap().to_owned();
    assert!(
        image.starts_with(&format!("{repository}@sha256:")),
        "{image}"
    );
    // A forged variant never runs: its signature covers another image.
    let forged = format!("{repository}@sha256:{}", "0".repeat(64));
    {
        let mut fake = fake.lock().unwrap();
        fake.config = json!({
            "app": app,
            "salt": "salt",
            "poll_interval_seconds": 1,
            "experiments": [{
                "name": "checkout",
                "version": 1,
                "subject": "request.cookies.ssp_sid",
                "eligibility": null,
                "salt": "experiment-salt",
                "traffic_ranges": [[0.0, 0.9]],
                "control": {"image": null, "address": null, "ready": false},
                "variants": [
                    {
                        "name": "lower-price",
                        "version": 1,
                        "ranges": [[0.0, 0.4]],
                        "image": image,
                        "commit": "abc123",
                        "signature": published["signature"],
                        "address": null,
                        "ready": false,
                    },
                    {
                        "name": "forged",
                        "version": 1,
                        "ranges": [[0.4, 0.5]],
                        "image": forged,
                        "commit": null,
                        "signature": published["signature"],
                        "address": null,
                        "ready": false,
                    },
                ],
            }],
        });
    }

    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let image_file = state.path().join("image");
    std::fs::write(&image_file, &default).unwrap();
    let start_proxy = || {
        Process(
            std::process::Command::new(cargo_bin("ssp"))
                .env("STATESPACE_CONFIG", config.path().join("config.toml"))
                .env("STATESPACE_URL", &api)
                .env("SSP_TOKEN", "test")
                .env("STATESPACE_STATE", state.path())
                .env("SHARED", "from-the-proxy")
                .args(["proxy", "--app", &app, "--runtime", "docker"])
                .arg("--image-file")
                .arg(&image_file)
                .args(["--trust-key", public_key])
                .args([
                    "--listen",
                    &format!("127.0.0.1:{port}"),
                    "--health",
                    "/health",
                ])
                .args(["--variant-env", "GREETING=variant"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut proxy = start_proxy();

    // Subjects reach the default, then each arm once it is ready.
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut served: std::collections::BTreeMap<String, Value> = Default::default();
    while !(served.contains_key("control") && served.contains_key("lower-price")) {
        assert!(
            Instant::now() < deadline,
            "the arms did not start: {served:?}"
        );
        for index in 0..40 {
            if let Some(body) = get(port, &format!("u_{index}")) {
                served.insert(body["variant"].as_str().unwrap_or("?").to_owned(), body);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(!served.contains_key("forged"), "{served:?}");
    assert!(
        !served.contains_key("spoofed"),
        "clients cannot pick an arm"
    );
    // Every container inherits the proxy's environment; arms also get --variant-env.
    assert_eq!(served["control"]["shared"], "from-the-proxy");
    assert_eq!(served["control"]["greeting"], "variant");
    assert_eq!(served["lower-price"]["greeting"], "variant");
    if let Some(default) = served.get("default") {
        assert_eq!(default["greeting"], "hello");
    }
    // A subject outside the experiment's slice gets the default, unrecorded.
    let outside = (0..200)
        .map(|index| format!("x_{index}"))
        .find_map(|subject| {
            let body = get(port, &subject)?;
            (body["variant"] == "default").then_some(body)
        })
        .expect("a subject outside the slice");
    assert_eq!(outside["greeting"], "hello");

    // The proxy pinned its own default for the control arm.
    let pin = fake.lock().unwrap().pin.clone().unwrap();
    assert!(pin.starts_with(&format!("{repository}@sha256:")), "{pin}");

    // Runs, the arms' requests, and every line the arms print reach Statespace.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let events = fake.lock().unwrap().events.clone();
        let rows = |table: &str| -> Vec<Value> {
            events
                .iter()
                .flat_map(|batch| batch[table].as_array().cloned().unwrap_or_default())
                .collect()
        };
        let output = rows("output");
        let visits = output
            .iter()
            .filter(|line| {
                line["line"]
                    .as_str()
                    .is_some_and(|line| line.contains("\"visit\""))
            })
            .count();
        let runs = rows("runs");
        let requests = rows("requests");
        if visits > 0 && !runs.is_empty() && !requests.is_empty() {
            assert!(output.iter().all(|line| line["experiment"] == "checkout"));
            assert!(
                requests
                    .iter()
                    .all(|request| request["variant"] != "default")
            );
            assert!(runs.iter().any(|run| run["variant"] == "control"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "events did not arrive: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    let reports = fake.lock().unwrap().reports.clone();
    assert!(
        reports
            .iter()
            .any(|report| report["kind"] == "proxy" && report["report"]["runtime"] == "docker")
    );

    // On SIGHUP the proxy swaps the default for the image its file names,
    // and every request is answered meanwhile.
    let outside_subject = (0..200)
        .map(|index| format!("x_{index}"))
        .find(|subject| get(port, subject).is_some_and(|body| body["variant"] == "default"))
        .unwrap();
    std::fs::write(&image_file, &variant).unwrap();
    let _ = std::process::Command::new("kill")
        .args(["-HUP", &proxy.0.id().to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let body = get(port, &outside_subject).expect("the proxy answers while it swaps");
        if body["greeting"] == "hi" {
            break;
        }
        assert!(Instant::now() < deadline, "the default was not swapped");
        std::thread::sleep(Duration::from_millis(200));
    }

    // After a crash, a new proxy adopts the containers that still run.
    let containers = || {
        let mut ids: Vec<String> = docker(&[
            "ps",
            "--quiet",
            "--filter",
            &format!("label=statespace.com/app={app}"),
        ])
        .split_whitespace()
        .map(str::to_owned)
        .collect();
        ids.sort();
        ids
    };
    // The replaced default stops once the new one serves.
    let deadline = Instant::now() + Duration::from_secs(30);
    let running = loop {
        let running = containers();
        if running.len() == 3 {
            break running;
        }
        assert!(
            Instant::now() < deadline,
            "the default and two arms: {running:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    proxy.0.kill().unwrap();
    proxy.0.wait().unwrap();
    let mut proxy = start_proxy();
    let deadline = Instant::now() + Duration::from_secs(60);
    while get(port, "u_0").is_none() {
        assert!(
            Instant::now() < deadline,
            "the restarted proxy did not listen"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        containers(),
        running,
        "the restarted proxy adopted every container"
    );

    // Stopping the proxy removes every container it ran.
    assert!(proxy.stop().is_some_and(|status| status.success()));
    let left = docker(&[
        "ps",
        "--all",
        "--quiet",
        "--filter",
        &format!("label=statespace.com/app={app}"),
    ]);
    assert!(left.is_empty(), "containers left: {left}");
}

fn rand_suffix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        & 0xff_ffff
}
