//! Pinned component build tools, downloaded on first use, verified by
//! SHA-256, and cached in the user cache directory.

use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

/// Every pinned tool version, recorded in each build's fingerprint.
pub const VERSIONS: &str = "componentize-py 0.25.1, jco 1.35.0, componentize-js 0.23.0, \
    wasm-tools 1.259.0, wit-bindgen 0.62.0, componentize-go 0.4.3, wasi-sdk 34.0";

const COMPONENTIZE_PY: &str = "0.25.1";
const JCO: &str = "1.35.0";
const COMPONENTIZE_JS: &str = "0.23.0";

struct Tool {
    name: &'static str,
    version: &'static str,
    repository: &'static str,
    tag: &'static str,
    /// `(os, arch, release asset, SHA-256)`.
    assets: &'static [(&'static str, &'static str, &'static str, &'static str)],
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "uv",
        version: "0.12.19",
        repository: "astral-sh/uv",
        tag: "0.12.19",
        assets: &[
            (
                "macos",
                "aarch64",
                "uv-aarch64-apple-darwin.tar.gz",
                "a9a8df1eedeb192f2e47e40e2faabfb387db4b850209118786d42f89dde3e0ba",
            ),
            (
                "macos",
                "x86_64",
                "uv-x86_64-apple-darwin.tar.gz",
                "cb5fa57bafe68fc0fb94b17f06bee0b0b9a7feb94ccbd110445afa0696e39273",
            ),
            (
                "linux",
                "aarch64",
                "uv-aarch64-unknown-linux-gnu.tar.gz",
                "0804e9b164c64b6914182d5920c08551958a095986f10a3731056df701126436",
            ),
            (
                "linux",
                "x86_64",
                "uv-x86_64-unknown-linux-gnu.tar.gz",
                "23bf5552d220e0842b65c862097b2ebaeba0064b74eda5e565e77fd25969d8c8",
            ),
            (
                "windows",
                "x86_64",
                "uv-x86_64-pc-windows-msvc.zip",
                "6dbb02d79e419522f1c500f0adb1cddcff0cda7d59b0d66ea7f5e3b4a1b2f5f0",
            ),
            (
                "windows",
                "aarch64",
                "uv-aarch64-pc-windows-msvc.zip",
                "115b54cb823bc48260670f5782001add6067ac8d98d18c8263a833704e287de9",
            ),
        ],
    },
    Tool {
        name: "wasm-tools",
        version: "1.259.0",
        repository: "bytecodealliance/wasm-tools",
        tag: "v1.259.0",
        assets: &[
            (
                "macos",
                "aarch64",
                "wasm-tools-1.259.0-aarch64-macos.tar.gz",
                "b662d939220b2c49ea9f1d81a19776e47d72bb5ba580f08107c7dc865e1f8c66",
            ),
            (
                "macos",
                "x86_64",
                "wasm-tools-1.259.0-x86_64-macos.tar.gz",
                "7ffdbb9f00207bd16e0612931c63ac1012255a0c805241b339dfe0cf39356a25",
            ),
            (
                "linux",
                "aarch64",
                "wasm-tools-1.259.0-aarch64-linux.tar.gz",
                "9e1644f3841f2b783dd8af82de054667388e46689806fba4909ed45f3182800f",
            ),
            (
                "linux",
                "x86_64",
                "wasm-tools-1.259.0-x86_64-linux.tar.gz",
                "3e9b374b4c7715b771b69bf0d65a337990ed4546ec5e97e01c0ff587dfc52160",
            ),
            (
                "windows",
                "aarch64",
                "wasm-tools-1.259.0-aarch64-windows.zip",
                "32f90b70a645a239171bc9e2f84e6605b5a83723e042c2da477cc83d17263502",
            ),
            (
                "windows",
                "x86_64",
                "wasm-tools-1.259.0-x86_64-windows.zip",
                "3c669d3706a5db7611a11afc422124df53a2c67294575e21f28c9b3deb95fa08",
            ),
        ],
    },
    Tool {
        name: "wit-bindgen",
        version: "0.62.0",
        repository: "bytecodealliance/wit-bindgen",
        tag: "v0.62.0",
        assets: &[
            (
                "macos",
                "aarch64",
                "wit-bindgen-0.62.0-aarch64-macos.tar.gz",
                "68a8898f8d139d24bd129c5206007dfb7636898edf7659348760d8159ecea9d6",
            ),
            (
                "macos",
                "x86_64",
                "wit-bindgen-0.62.0-x86_64-macos.tar.gz",
                "0fe161319d31be62e2a39c8786772230a36e120be457d67c34b67706de91911b",
            ),
            (
                "linux",
                "aarch64",
                "wit-bindgen-0.62.0-aarch64-linux.tar.gz",
                "16829d8e4b81ef381c7007ad94d2b14c365360d0024cdfb5a34e5fbd4d8f1d89",
            ),
            (
                "linux",
                "x86_64",
                "wit-bindgen-0.62.0-x86_64-linux.tar.gz",
                "3e81cc6523729f7532b4aa7968648a04abf0c711b7d1677150e9121f4e6458fe",
            ),
            (
                "windows",
                "aarch64",
                "wit-bindgen-0.62.0-aarch64-windows.zip",
                "c9cccf5ecbf51f0e7dd12d38654c9dbe8aef433711d3c52ffdbb82c22e8a1209",
            ),
            (
                "windows",
                "x86_64",
                "wit-bindgen-0.62.0-x86_64-windows.zip",
                "9c9ffe30e8043cf162df53c5d98818e157f9f985d8248a0c2596812b50f0f183",
            ),
        ],
    },
    Tool {
        name: "componentize-go",
        version: "0.4.3",
        repository: "bytecodealliance/componentize-go",
        tag: "v0.4.3",
        assets: &[
            (
                "macos",
                "aarch64",
                "componentize-go-darwin-arm64.tar.gz",
                "340c26062eb5079fe32fbc325baf5421df7b923de86ef110b8592ee881e4a6cf",
            ),
            (
                "macos",
                "x86_64",
                "componentize-go-darwin-amd64.tar.gz",
                "0f562d5a98355c6988ea4485e87ae6c08d87fe7423f3ee9252ea8ee2d9d3f77f",
            ),
            (
                "linux",
                "aarch64",
                "componentize-go-linux-arm64.tar.gz",
                "442b528560bdbe47e4894c228d658d3d4ecc7b4f42516dff7f41cf7770732d82",
            ),
            (
                "linux",
                "x86_64",
                "componentize-go-linux-amd64.tar.gz",
                "1061d845f550df5d9477612a7d458a31b1e2b8bdc95823704e42e5064deb0c52",
            ),
            (
                "windows",
                "x86_64",
                "componentize-go-windows-amd64.zip",
                "b7fc1dbfdb9edb97d06d94124b845363bab14ff6a819b518dd6329d0a9428567",
            ),
        ],
    },
    Tool {
        name: "wasi-sdk",
        version: "34.0",
        repository: "WebAssembly/wasi-sdk",
        tag: "wasi-sdk-34",
        assets: &[
            (
                "macos",
                "aarch64",
                "wasi-sdk-34.0-arm64-macos.tar.gz",
                "9c59398106b417f8f14913380fdf0097a8cc0ff4af9eb3ce0065a859e88d49e9",
            ),
            (
                "macos",
                "x86_64",
                "wasi-sdk-34.0-x86_64-macos.tar.gz",
                "87d27fa8adc68dee59bfbf2e22a6d34ef717c34d6bf1d8af2a56fc929d9ce0eb",
            ),
            (
                "linux",
                "aarch64",
                "wasi-sdk-34.0-arm64-linux.tar.gz",
                "f7e243dff54d60bcc576e94d6166b69f410f2500ae4a9ceef34315be10e77971",
            ),
            (
                "linux",
                "x86_64",
                "wasi-sdk-34.0-x86_64-linux.tar.gz",
                "b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4",
            ),
            (
                "windows",
                "aarch64",
                "wasi-sdk-34.0-arm64-windows.tar.gz",
                "45e1c71f3e965621e7b98ebe1d37b0e4b1f77f3e8072113ffb4534e67b1a4b7c",
            ),
            (
                "windows",
                "x86_64",
                "wasi-sdk-34.0-x86_64-windows.tar.gz",
                "cccb5c323a9b34f0349a9b09e8804a0a7632c68c3310f4b5f437ed57d7e71d8f",
            ),
        ],
    },
];

fn tool(name: &str) -> &'static Tool {
    TOOLS
        .iter()
        .find(|tool| tool.name == name)
        .expect("the tool is pinned")
}

pub fn cache() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("STATESPACE_TOOL_CACHE") {
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::cache_dir()
        .context("the user cache directory is unavailable")?
        .join("statespace/tools"))
}

fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

/// Download and verify the release archive for this platform.
fn download(tool: &Tool, directory: &Path) -> anyhow::Result<(PathBuf, bool)> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => bail!("component builds are not supported on {other}"),
    };
    let arch = std::env::consts::ARCH;
    let (_, _, asset, checksum) = tool
        .assets
        .iter()
        .find(|(asset_os, asset_arch, _, _)| *asset_os == os && *asset_arch == arch)
        .with_context(|| format!("{} is not available for {os}/{arch}", tool.name))?;
    let url = format!(
        "https://github.com/{}/releases/download/{}/{asset}",
        tool.repository, tool.tag
    );
    eprintln!("Downloading {} {}", tool.name, tool.version);
    let mut response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?
        .get(&url)
        .send()?
        .error_for_status()?;
    let archive = directory.join("download");
    io::copy(&mut response, &mut File::create(&archive)?)?;
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(&archive)?, &mut hasher)?;
    if format!("{:x}", hasher.finalize()) != *checksum {
        bail!("checksum mismatch for {url}");
    }
    Ok((archive, asset.ends_with(".zip")))
}

/// Return a pinned single-binary tool, installing it on first use.
pub fn binary(name: &str) -> anyhow::Result<PathBuf> {
    let tool = tool(name);
    let directory = cache()?.join(format!("{}-{}", tool.name, tool.version));
    let binary = directory.join(executable(name));
    if binary.is_file() {
        return Ok(binary);
    }
    fs::create_dir_all(cache()?)?;
    let staging = tempfile::tempdir_in(cache()?)?;
    let (archive, zip) = download(tool, staging.path())?;
    extract(
        &archive,
        zip,
        &executable(name),
        &staging.path().join(executable(name)),
    )?;
    fs::remove_file(&archive)?;
    install(staging.path(), &directory)?;
    Ok(binary)
}

/// Return the WASI SDK, installing it on first use. `WASI_SDK_PATH` overrides it.
pub fn wasi_sdk() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("WASI_SDK_PATH") {
        return Ok(PathBuf::from(path));
    }
    let tool = tool("wasi-sdk");
    let directory = cache()?.join(format!("wasi-sdk-{}", tool.version));
    if directory.join("bin").join(executable("clang")).is_file() {
        return Ok(directory);
    }
    fs::create_dir_all(cache()?)?;
    let staging = tempfile::tempdir_in(cache()?)?;
    let (archive, _) = download(tool, staging.path())?;
    let unpacked = staging.path().join("unpacked");
    tar::Archive::new(flate2::read::GzDecoder::new(File::open(&archive)?)).unpack(&unpacked)?;
    let root = fs::read_dir(&unpacked)?
        .next()
        .context("the WASI SDK archive is empty")??
        .path();
    install(&root, &directory)?;
    Ok(directory)
}

/// Run componentize-py through a pinned uv.
pub fn componentize_py() -> anyhow::Result<Command> {
    let mut command = Command::new(binary("uv")?);
    command
        .env("UV_CACHE_DIR", cache()?.join("uv-cache"))
        .args(["tool", "run", "--from"])
        .arg(format!("componentize-py=={COMPONENTIZE_PY}"))
        .arg("componentize-py");
    Ok(command)
}

/// Run jco with ComponentizeJS, installed with npm on first use.
pub fn jco() -> anyhow::Result<Command> {
    let root = cache()?.join(format!("jco-{JCO}-componentize-js-{COMPONENTIZE_JS}"));
    let jco = root
        .join("node_modules/.bin")
        .join(if cfg!(windows) { "jco.cmd" } else { "jco" });
    if !jco.is_file() {
        fs::create_dir_all(&root)?;
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        super::run(
            Command::new(npm)
                .args(["install", "--no-audit", "--no-fund", "--prefix"])
                .arg(&root)
                .arg(format!("@bytecodealliance/jco@{JCO}"))
                .arg(format!(
                    "@bytecodealliance/componentize-js@{COMPONENTIZE_JS}"
                )),
            "npm install (Node.js is required for JavaScript builds)",
        )?;
    }
    Ok(Command::new(jco))
}

pub fn ensure_rust_target(target: &str) -> anyhow::Result<()> {
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("rustup is required for Rust builds")?;
    if String::from_utf8_lossy(&installed.stdout)
        .lines()
        .any(|line| line == target)
    {
        return Ok(());
    }
    super::run(
        Command::new("rustup").args(["target", "add", target]),
        "rustup target add",
    )
}

fn extract(archive: &Path, zip: bool, name: &str, destination: &Path) -> anyhow::Result<()> {
    let found = if zip {
        let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
        let index = (0..archive.len()).find(|&index| {
            archive.by_index(index).is_ok_and(|entry| {
                Path::new(entry.name()).file_name() == Some(std::ffi::OsStr::new(name))
            })
        });
        match index {
            Some(index) => {
                io::copy(
                    &mut archive.by_index(index)?,
                    &mut File::create(destination)?,
                )?;
                true
            }
            None => false,
        }
    } else {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
        let mut found = false;
        for entry in archive.entries()? {
            let mut entry = entry?;
            if entry.header().entry_type().is_file()
                && entry.path()?.file_name() == Some(std::ffi::OsStr::new(name))
            {
                io::copy(&mut entry, &mut File::create(destination)?)?;
                found = true;
                break;
            }
        }
        found
    };
    if !found {
        bail!("the archive does not contain {name}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Move a staged installation into place. Another process may win the race.
fn install(staged: &Path, directory: &Path) -> anyhow::Result<()> {
    match fs::rename(staged, directory) {
        Ok(()) => Ok(()),
        Err(_) if directory.exists() => Ok(()),
        Err(error) => Err(error.into()),
    }
}
