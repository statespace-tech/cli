//! Checks of the `ssp` binary that need no Statespace server.

use std::{
    net::TcpListener,
    path::Path,
    process::{Child, Stdio},
    time::{Duration, Instant},
};

use assert_cmd::{Command, cargo::cargo_bin};
use serde_json::Value;

/// Run `ssp` with an empty configuration so no saved session is used.
fn ssp(arguments: &[&str]) -> (bool, Value, Value) {
    let config = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("ssp")
        .unwrap()
        .env("STATESPACE_CONFIG", config.path().join("config.toml"))
        .env_remove("SSP_API_KEY")
        .env_remove("SSP_SIGNING_KEY")
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
    let key = "--sign-key=not-a-key";
    let message = error(&["variant", "create", "fast", ".", "--app", "web", key]);
    assert!(message.contains("signing key"), "{message}");
}

#[test]
fn prints_signing_key_pairs() {
    let (ok, keys, _) = ssp(&["variant", "keygen"]);
    assert!(ok);
    assert_eq!(keys["private_key"].as_str().unwrap().len(), 44);
    assert_eq!(keys["public_key"].as_str().unwrap().len(), 44);
}

#[test]
fn hosts_need_a_port_in_their_command() {
    let message = error(&["host", "--local", "--command", "python3 app.py"]);
    assert!(message.contains("{port}"), "{message}");
}

/// Stops the host when a test ends, even when it fails.
struct Host(Child);

impl Host {
    /// Ask the host to stop, as a platform does, and wait for it to exit.
    fn stop(&mut self) -> Option<std::process::ExitStatus> {
        #[cfg(unix)]
        // SAFETY: signalling a process this test started.
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.0.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if self.stop().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn start_host(directory: &Path, port: u16, command: &str, arguments: &[String]) -> Host {
    let child = std::process::Command::new(cargo_bin("ssp"))
        .current_dir(directory)
        .env_remove("SSP_API_KEY")
        .env_remove("STATESPACE_APP")
        .args(["host", "--local", "--listen", &format!("127.0.0.1:{port}")])
        .args(["--command", command])
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Host(child)
}

fn get(port: u16, headers: &[(&str, &str)]) -> Option<(Value, Vec<String>)> {
    let mut request = reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{port}/checkout?step=1"))
        .timeout(Duration::from_secs(5));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().ok()?;
    let cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect();
    Some((response.json().ok()?, cookies))
}

fn has_python() -> bool {
    let found = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok();
    if !found {
        eprintln!("skipped: python3 is not installed");
    }
    found
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn local_hosts_route_requests_to_variants() {
    if !has_python() {
        return;
    }
    let control = tempfile::tempdir().unwrap();
    let variant = tempfile::tempdir().unwrap();
    for directory in [control.path(), variant.path()] {
        std::fs::copy("examples/app/app.py", directory.join("app.py")).unwrap();
    }
    let port = free_port();
    let mut host = start_host(
        control.path(),
        port,
        "python3 app.py {port}",
        &[
            "--variant-env".into(),
            "GREETING=hi".into(),
            "--variant".into(),
            format!("fast={}", variant.path().display()),
        ],
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    let (body, cookies) = loop {
        if let Some(response) = get(port, &[]) {
            break response;
        }
        assert!(Instant::now() < deadline, "the host did not start");
        std::thread::sleep(Duration::from_millis(200));
    };
    // Control runs the deployed code with the host's environment.
    assert_eq!(body["variant"], "control");
    assert_eq!(body["greeting"], "hello");
    // A new visitor gets a subject cookie, which the app sees.
    let cookie = cookies
        .iter()
        .find(|cookie| cookie.starts_with("ssp_sid="))
        .expect("a subject cookie");
    let subject = cookie["ssp_sid=".len()..].split(';').next().unwrap();
    assert_eq!(body["subject"], subject);

    // A known visitor keeps its subject, and clients cannot set Statespace headers.
    let (body, cookies) = loop {
        // The variant may still be starting.
        let (body, cookies) = get(
            port,
            &[
                ("cookie", "ssp_sid=u_42"),
                ("x-statespace-variant", "fast"),
                ("x-statespace-subject", "spoofed"),
            ],
        )
        .unwrap();
        if body["variant"] == "fast" || Instant::now() > deadline {
            break (body, cookies);
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(body["variant"], "fast");
    assert_eq!(body["subject"], "u_42");
    assert_eq!(body["greeting"], "hi", "variants get --variant-env");
    assert!(cookies.iter().all(|cookie| !cookie.starts_with("ssp_sid=")));

    // Clients may speak HTTP/2 to the host.
    let http2 = reqwest::blocking::Client::builder()
        .http2_prior_knowledge()
        .build()
        .unwrap()
        .get(format!("http://127.0.0.1:{port}/"))
        .header("cookie", "ssp_sid=u_42")
        .send()
        .unwrap();
    assert_eq!(http2.version(), reqwest::Version::HTTP_2);
    assert_eq!(http2.json::<Value>().unwrap()["subject"], "u_42");

    // Stopping the host stops the app's processes too.
    assert!(host.stop().is_some_and(|status| status.success()));
    assert!(get(port, &[]).is_none());
}

/// Whether a process with this ID is alive.
#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
fn read_pid(path: &Path, deadline: Instant) -> i32 {
    loop {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "{} was not written",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
#[test]
fn restarts_leave_no_workers_behind() {
    if !has_python() {
        return;
    }
    let control = tempfile::tempdir().unwrap();
    std::fs::copy("examples/app/app.py", control.path().join("app.py")).unwrap();
    let port = free_port();
    // Like a server that forks workers: one outlives the process that crashes.
    let mut host = start_host(
        control.path(),
        port,
        "sleep 300 & echo $! > worker.pid; echo $$ > app.pid; exec python3 app.py {port}",
        &[],
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let worker = read_pid(&control.path().join("worker.pid"), deadline);
    let app = read_pid(&control.path().join("app.pid"), deadline);
    std::fs::remove_file(control.path().join("app.pid")).unwrap();
    // SAFETY: killing a process this test's host started.
    unsafe {
        libc::kill(app, libc::SIGKILL);
    }
    while alive(worker) {
        assert!(
            Instant::now() < deadline,
            "the crashed process's worker survived"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // The host restarts the app.
    let restarted = read_pid(&control.path().join("app.pid"), deadline);
    assert_ne!(restarted, app);
    while get(port, &[]).is_none() {
        assert!(Instant::now() < deadline, "the app did not restart");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(host.stop().is_some_and(|status| status.success()));
}
