//! Offline checks of the `ssp` binary.

use assert_cmd::Command;
use serde_json::Value;

/// Run `ssp` with an empty configuration so no saved session is used.
fn ssp(arguments: &[&str]) -> (bool, Value, Value) {
    let config = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("ssp")
        .unwrap()
        .env("STATESPACE_CONFIG", config.path().join("config.toml"))
        .env_remove("SSP_API_KEY")
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

#[test]
fn reports_errors_as_json() {
    let (ok, _, error) = ssp(&["experiment", "list"]);
    assert!(!ok);
    assert!(error["error"].as_str().unwrap().contains("ssp login"));

    let (ok, _, error) = ssp(&["experiment", "delete", "ranking"]);
    assert!(!ok);
    assert!(error["error"].as_str().unwrap().contains("--yes"));
}
