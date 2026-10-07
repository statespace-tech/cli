//! Resolve `NAME=VALUE` arguments into parameters.
//!
//! A value is, in order:
//! 1. an existing file or directory, optionally followed by `:entry`:
//!    - a source file or Go or Rust project, built into a component;
//!    - a `.json` file, parsed as JSON;
//!    - any other file, used as text;
//! 2. a JSON literal.
//!
//! Anything else is an error, so a mistyped file name never becomes a string.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{api::Api, build, wasm};

/// A function named on the command line.
pub struct FunctionReference {
    /// The path as written, recorded as the function's origin.
    pub path: String,
    pub file: PathBuf,
    pub entry: Option<String>,
}

/// A component ready to upload or run.
pub struct Component {
    pub bytes: Vec<u8>,
    pub sha256: String,
    /// The origin recorded with the parameter.
    pub source: Value,
}

enum Resolved {
    Value(Value),
    Function(FunctionReference),
}

/// Resolve `NAME=VALUE` arguments, uploading functions that the account does
/// not have yet. An empty value maps to `null`, which removes a parameter.
pub fn resolve(api: &Api, arguments: &[String]) -> anyhow::Result<serde_json::Map<String, Value>> {
    let mut parameters = serde_json::Map::new();
    for argument in arguments {
        let (name, raw) = argument
            .split_once('=')
            .with_context(|| format!("{argument:?} must look like NAME=VALUE"))?;
        if name.is_empty() || parameters.contains_key(name) {
            bail!("{argument:?} has an empty or repeated parameter name");
        }
        let parameter = match raw {
            "" => Value::Null,
            raw => match classify(raw)? {
                Resolved::Value(value) => json!({"kind": "value", "value": value}),
                Resolved::Function(reference) => {
                    let component = prepare(&reference)?;
                    upload(api, &component)?;
                    json!({"kind": "function", "sha256": component.sha256, "source": component.source})
                }
            },
        };
        parameters.insert(name.into(), parameter);
    }
    Ok(parameters)
}

/// Parse `path[:entry]` when it names an existing file or directory.
pub fn function_reference(raw: &str) -> Option<FunctionReference> {
    let reference = |path: &str, entry: Option<&str>| FunctionReference {
        path: path.into(),
        file: PathBuf::from(path),
        entry: entry.map(Into::into),
    };
    if Path::new(raw).exists() {
        return Some(reference(raw, None));
    }
    let (path, entry) = raw.rsplit_once(':')?;
    Path::new(path)
        .exists()
        .then(|| reference(path, Some(entry)))
}

fn classify(raw: &str) -> anyhow::Result<Resolved> {
    if let Some(reference) = function_reference(raw) {
        let extension = reference
            .file
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if extension == "wasm" {
            bail!(prebuilt(&reference));
        }
        if reference.file.is_dir() || build::is_source(&reference.file) {
            return Ok(Resolved::Function(reference));
        }
        if reference.entry.is_some() {
            bail!("{raw:?}: only functions take an :entry");
        }
        let text = fs::read_to_string(&reference.file)
            .with_context(|| format!("{} is not a UTF-8 text file", reference.path))?;
        return Ok(Resolved::Value(if extension == "json" {
            serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid JSON", reference.path))?
        } else {
            Value::String(text)
        }));
    }
    serde_json::from_str(raw).map(Resolved::Value).map_err(|_| {
        anyhow::anyhow!(
            "{raw:?} is not a file or valid JSON; quote strings as JSON, such as '\"{raw}\"'"
        )
    })
}

/// Build a component from source. A build is identified by its source and
/// cached, so unchanged code is neither rebuilt nor re-versioned.
pub fn prepare(reference: &FunctionReference) -> anyhow::Result<Component> {
    if reference
        .file
        .extension()
        .is_some_and(|value| value == "wasm")
    {
        bail!(prebuilt(reference));
    }
    let entry = match &reference.entry {
        Some(entry) => entry.clone(),
        None if reference.file.is_file() => stem(&reference.file)?,
        None => bail!(
            "{}: name the function, such as {}:Score",
            reference.path,
            reference.path
        ),
    };
    let file = reference
        .file
        .canonicalize()
        .with_context(|| format!("{} does not exist", reference.path))?;
    let (source_sha256, code) = fingerprint(&file, &entry)?;
    let cached = build::cache_path(&source_sha256)?;
    if !cached.is_file() {
        build::build(&file, &entry, &cached)?;
    }
    let bytes = fs::read(&cached)?;
    Ok(Component {
        sha256: wasm::inspect(&bytes)?,
        source: json!({
            "path": reference.path,
            "entry": entry,
            "sha256": source_sha256,
            "code": code,
        }),
        bytes,
    })
}

/// Statespace builds every function itself, so its source and build are known.
fn prebuilt(reference: &FunctionReference) -> String {
    format!(
        "{} is a prebuilt component; pass its source instead, such as rerank.py:score",
        reference.path
    )
}

/// Upload a component unless the account already has it. Uploads are
/// idempotent, so transport failures, such as a connection dropped mid-upload,
/// are retried with backoff.
fn upload(api: &Api, component: &Component) -> anyhow::Result<()> {
    const ATTEMPTS: u32 = 5;
    let path = format!("/v1/artifacts/{}", component.sha256);
    let mut attempt = 1;
    loop {
        match try_upload(api, &path, component) {
            Err(error) if attempt < ATTEMPTS && is_transport_error(&error) => {
                std::thread::sleep(std::time::Duration::from_secs(1 << attempt));
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn try_upload(api: &Api, path: &str, component: &Component) -> anyhow::Result<()> {
    let status = api.request(Method::GET, path)?.send()?.status();
    if status == StatusCode::NOT_FOUND {
        crate::api::decode::<Value>(
            api.request(Method::PUT, path)?
                .header(reqwest::header::CONTENT_TYPE, "application/wasm")
                .body(component.bytes.clone())
                .send()?,
        )?;
    } else if !status.is_success() {
        bail!("could not check {}: {status}", component.sha256);
    }
    Ok(())
}

/// A request that failed before an HTTP response arrived.
fn is_transport_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| !error.is_status() && !error.is_decode() && !error.is_builder())
}

/// Hash everything a build depends on: the entry, the pinned tools, and the
/// source files. Returns the hash and, for a single file, its text.
fn fingerprint(source: &Path, entry: &str) -> anyhow::Result<(String, Option<String>)> {
    let mut hasher = Sha256::new();
    hasher.update(build::TOOLCHAIN.as_bytes());
    hasher.update(entry.as_bytes());
    let files = build::inputs(source)?;
    let root = build::project_directory(source)?;
    for file in &files {
        let relative = file.strip_prefix(&root).unwrap_or(file);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(fs::read(file)?);
    }
    let code = source
        .is_file()
        .then(|| fs::read_to_string(source).ok())
        .flatten()
        .filter(|code| code.len() <= 256 * 1024);
    Ok((format!("{:x}", hasher.finalize()), code))
}

fn stem(path: &Path) -> anyhow::Result<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(Into::into)
        .context("the source has no name")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(raw: &str) -> Value {
        match classify(raw).unwrap() {
            Resolved::Value(value) => value,
            Resolved::Function(_) => panic!("{raw} is not a value"),
        }
    }

    #[test]
    fn resolves_literals_and_files() {
        assert_eq!(value("20"), json!(20));
        assert_eq!(value("true"), json!(true));
        assert_eq!(value(r#""claude""#), json!("claude"));
        assert_eq!(value(r#"{"t":0.2}"#), json!({"t": 0.2}));
        let directory = tempfile::tempdir().unwrap();
        let prompt = directory.path().join("prompt.md");
        fs::write(&prompt, "Be concise.").unwrap();
        assert_eq!(value(prompt.to_str().unwrap()), json!("Be concise."));
        let config = directory.path().join("sampling.json");
        fs::write(&config, r#"{"top_p": 0.9}"#).unwrap();
        assert_eq!(value(config.to_str().unwrap()), json!({"top_p": 0.9}));
    }

    #[test]
    fn rejects_words_that_are_not_files_or_json() {
        let error = classify("prompts/v2.mx").err().unwrap().to_string();
        assert!(error.contains("not a file or valid JSON"));
    }

    #[test]
    fn recognizes_functions_with_entries() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("rerank.py");
        fs::write(&source, "def score(items):\n    return items\n").unwrap();
        let raw = format!("{}:score", source.display());
        let Resolved::Function(reference) = classify(&raw).unwrap() else {
            panic!("a Python file is a function");
        };
        assert_eq!(reference.entry.as_deref(), Some("score"));
        assert_eq!(
            fingerprint(&source, "score").unwrap().1.unwrap(),
            "def score(items):\n    return items\n"
        );
    }
}
