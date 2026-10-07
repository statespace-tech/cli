//! Compile functions into Statespace components with each language's
//! standard component tooling. Tools are pinned, checksum-verified, and
//! cached on first use.

mod c;
mod go;
mod javascript;
mod python;
mod rust;
mod toolchain;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};

use crate::wasm;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Python,
    Javascript,
    Typescript,
    Go,
    Rust,
    C,
    Cpp,
}

/// The pinned tool versions. Part of every build's identity.
pub const TOOLCHAIN: &str = toolchain::VERSIONS;

/// Directories that never affect a build.
const IGNORED: [&str; 7] = [
    ".git",
    ".venv",
    "__pycache__",
    "node_modules",
    "target",
    "dist",
    "build",
];

/// The function that a build exports.
pub struct Entry<'a> {
    /// A module, file, or package, when the entry names one.
    pub module: Option<&'a str>,
    pub function: &'a str,
}

/// Whether a file is source code that builds into a function.
pub fn is_source(path: &Path) -> bool {
    path.is_file() && infer_language(path).is_ok()
}

/// Build `source` into a validated component at `output`.
pub fn build(source: &Path, entry: &str, output: &Path) -> anyhow::Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("{} does not exist", source.display()))?;
    let language = infer_language(&source)?;
    let entry = parse_entry(entry, language)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    // Build next to the destination, then move into place, so a cache entry
    // is never half written.
    let staged = tempfile::NamedTempFile::new_in(output.parent().unwrap_or(Path::new(".")))?;
    let workspace = tempfile::tempdir()?;
    let wit = workspace.path().join("statespace.wit");
    fs::write(&wit, wasm::WIT)?;
    let target = staged.path();
    eprintln!("Building {}", source.display());
    match language {
        Language::Python => python::build(&source, &entry, &wit, target)?,
        Language::Javascript | Language::Typescript => {
            javascript::build(&source, &entry, &wit, target)?;
        }
        Language::Go => go::build(&source, &entry, &wit, target)?,
        Language::Rust => rust::build(&source, &entry, &wit, target)?,
        Language::C | Language::Cpp => c::build(&source, &entry, language, &wit, target)?,
    }
    wasm::inspect(&fs::read(target)?)?;
    staged.persist(output)?;
    Ok(())
}

/// Where a build with this source fingerprint is cached.
pub fn cache_path(fingerprint: &str) -> anyhow::Result<PathBuf> {
    Ok(toolchain::cache()?
        .join("builds")
        .join(format!("{fingerprint}.wasm")))
}

/// Where a compiled component is cached. Compiled code is valid only for the
/// Wasmtime version that wrote it, and each CLI release pins one version.
pub fn compiled_path(sha256: &str) -> anyhow::Result<PathBuf> {
    let version = env!("CARGO_PKG_VERSION");
    Ok(toolchain::cache()?
        .join("compiled")
        .join(format!("{sha256}-ssp-{version}.cwasm")))
}

/// The files a build reads: every file of a project directory, or a source
/// file with the sibling files in its language that it may import.
pub fn inputs(source: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if source.is_dir() {
        collect(source, &mut files)?;
    } else {
        let language = infer_language(source)?;
        for item in fs::read_dir(project_directory(source)?)? {
            let path = item?.path();
            if path.is_file() && infer_language(&path).ok() == Some(language) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn collect(directory: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for item in fs::read_dir(directory)? {
        let path = item?.path();
        let ignored = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| IGNORED.contains(&name));
        if ignored {
            continue;
        }
        if path.is_dir() {
            collect(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn infer_language(source: &Path) -> anyhow::Result<Language> {
    if source.is_dir() {
        if source.join("go.mod").is_file() {
            return Ok(Language::Go);
        }
        if source.join("Cargo.toml").is_file() {
            return Ok(Language::Rust);
        }
        bail!("a function directory must be a Go module or a Rust crate");
    }
    let extension = source.extension().and_then(|value| value.to_str());
    Ok(match extension {
        Some("py") => Language::Python,
        Some("js" | "mjs") => Language::Javascript,
        Some("ts" | "mts") => Language::Typescript,
        Some("c") => Language::C,
        Some("cc" | "cpp" | "cxx") => Language::Cpp,
        _ => bail!(
            "cannot infer the language of {}; pass --language",
            source.display()
        ),
    })
}

fn parse_entry(entry: &str, language: Language) -> anyhow::Result<Entry<'_>> {
    // Rust paths use `::`, so only other languages split on a single colon.
    let (module, function) = match entry.rsplit_once(':') {
        Some((module, function)) if language != Language::Rust => (Some(module), function),
        _ => (None, entry),
    };
    let valid = match language {
        Language::Rust => entry.split("::").all(identifier),
        _ => identifier(function) && module.is_none_or(|module| !module.is_empty()),
    };
    if !valid {
        bail!("invalid entry {entry:?}");
    }
    Ok(Entry { module, function })
}

pub(crate) fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// The directory that contains `source`, or `source` itself.
pub(crate) fn project_directory(source: &Path) -> anyhow::Result<PathBuf> {
    if source.is_dir() {
        return Ok(source.to_path_buf());
    }
    Ok(source.parent().context("the source has no parent")?.into())
}

pub(crate) fn run(command: &mut Command, name: &str) -> anyhow::Result<()> {
    let output = command
        .output()
        .with_context(|| format!("could not run {name}"))?;
    if !output.status.success() {
        bail!(
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_languages_from_sources() {
        for (file, language) in [
            ("rank.py", Language::Python),
            ("rank.ts", Language::Typescript),
            ("rank.mjs", Language::Javascript),
            ("rank.cpp", Language::Cpp),
        ] {
            assert_eq!(infer_language(Path::new(file)).unwrap(), language);
        }
        assert!(infer_language(Path::new("rank.txt")).is_err());
    }

    #[test]
    fn parses_entries() {
        let entry = parse_entry("ranking.models:score", Language::Python).unwrap();
        assert_eq!(
            (entry.module, entry.function),
            (Some("ranking.models"), "score")
        );
        let entry = parse_entry("rank", Language::Python).unwrap();
        assert_eq!((entry.module, entry.function), (None, "rank"));
        assert!(parse_entry("ranking::score", Language::Rust).is_ok());
        assert!(parse_entry("1rank", Language::C).is_err());
        assert!(parse_entry(":rank", Language::Go).is_err());
    }
}
