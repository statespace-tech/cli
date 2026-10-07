//! Rust with wit-bindgen and wasm-tools.

use std::{fs, path::Path, process::Command};

use anyhow::{Context, bail};

use super::{Entry, run, toolchain};

pub fn build(source: &Path, entry: &Entry, wit: &Path, output: &Path) -> anyhow::Result<()> {
    let manifest: toml::Table = fs::read_to_string(source.join("Cargo.toml"))
        .context("a Rust source must be a crate directory with Cargo.toml")?
        .parse()?;
    let package = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .context("Cargo.toml has no package name")?;
    if !source.join("src/lib.rs").is_file() {
        bail!("the Rust crate must be a library with src/lib.rs");
    }
    // A wrapper crate depends on the user crate and exports `execute`.
    let workspace = tempfile::tempdir()?;
    fs::create_dir_all(workspace.path().join("src"))?;
    fs::create_dir_all(workspace.path().join("wit"))?;
    fs::copy(wit, workspace.path().join("wit/world.wit"))?;
    fs::write(
        workspace.path().join("Cargo.toml"),
        format!(
            r#"[package]
name = "statespace_component"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde_json = "1"
wit-bindgen = "0.49"
user_component = {{ package = {package:?}, path = {path:?} }}
"#,
            path = source.to_string_lossy(),
        ),
    )?;
    fs::write(
        workspace.path().join("src/lib.rs"),
        format!(
            r#"wit_bindgen::generate!({{ path: "wit", world: "statespace" }});

struct Component;

impl Guest for Component {{
    fn execute(input: String) -> String {{
        let input = serde_json::from_str(&input).expect("valid JSON input");
        serde_json::to_string(&user_component::{function}(input)).expect("serializable output")
    }}
}}

export!(Component);
"#,
            function = entry.function,
        ),
    )?;
    toolchain::ensure_rust_target("wasm32-unknown-unknown")?;
    run(
        Command::new("cargo").current_dir(workspace.path()).args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ]),
        "cargo build",
    )?;
    let module = workspace
        .path()
        .join("target/wasm32-unknown-unknown/release/statespace_component.wasm");
    run(
        Command::new(toolchain::binary("wasm-tools")?)
            .args(["component", "new"])
            .arg(module)
            .arg("-o")
            .arg(output),
        "wasm-tools component new",
    )
}
