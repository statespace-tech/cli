//! Installs pinned component build tools in the user's cache on first use.

use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

const UV_VERSION: &str = "0.12.19";
const PY_VERSION: &str = "0.25.1";
const JCO_VERSION: &str = "1.35.0";
const COMPONENTIZE_JS_VERSION: &str = "0.23.0";

struct Release {
    url: String,
    sha256: &'static str,
}

fn platform() -> anyhow::Result<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(("macos", "aarch64")),
        ("macos", "x86_64") => Ok(("macos", "x86_64")),
        ("linux", "aarch64") => Ok(("linux", "aarch64")),
        ("linux", "x86_64") => Ok(("linux", "x86_64")),
        ("windows", "x86_64") => Ok(("windows", "x86_64")),
        ("windows", "aarch64") => Ok(("windows", "aarch64")),
        (os, arch) => bail!("component builds are not supported on {os}/{arch}"),
    }
}

fn cache() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("STATESPACE_TOOL_CACHE") {
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::cache_dir()
        .context("could not find a user cache directory")?
        .join("statespace/component-tools"))
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn release(name: &str) -> anyhow::Result<Release> {
    let (os, arch) = platform()?;
    let (version, repository, asset, sha256) = match (name, os, arch) {
        ("uv", "macos", "aarch64") => (
            UV_VERSION,
            "astral-sh/uv",
            "uv-aarch64-apple-darwin.tar.gz",
            "a9a8df1eedeb192f2e47e40e2faabfb387db4b850209118786d42f89dde3e0ba",
        ),
        ("uv", "macos", "x86_64") => (
            UV_VERSION,
            "astral-sh/uv",
            "uv-x86_64-apple-darwin.tar.gz",
            "cb5fa57bafe68fc0fb94b17f06bee0b0b9a7feb94ccbd110445afa0696e39273",
        ),
        ("uv", "linux", "aarch64") => (
            UV_VERSION,
            "astral-sh/uv",
            "uv-aarch64-unknown-linux-gnu.tar.gz",
            "0804e9b164c64b6914182d5920c08551958a095986f10a3731056df701126436",
        ),
        ("uv", "linux", "x86_64") => (
            UV_VERSION,
            "astral-sh/uv",
            "uv-x86_64-unknown-linux-gnu.tar.gz",
            "23bf5552d220e0842b65c862097b2ebaeba0064b74eda5e565e77fd25969d8c8",
        ),
        ("uv", "windows", "x86_64") => (
            UV_VERSION,
            "astral-sh/uv",
            "uv-x86_64-pc-windows-msvc.zip",
            "6dbb02d79e419522f1c500f0adb1cddcff0cda7d59b0d66ea7f5e3b4a1b2f5f0",
        ),
        ("uv", "windows", "aarch64") => (
            UV_VERSION,
            "astral-sh/uv",
            "uv-aarch64-pc-windows-msvc.zip",
            "115b54cb823bc48260670f5782001add6067ac8d98d18c8263a833704e287de9",
        ),
        ("wasm-tools", "macos", "aarch64") => (
            "1.259.0",
            "bytecodealliance/wasm-tools",
            "wasm-tools-1.259.0-aarch64-macos.tar.gz",
            "b662d939220b2c49ea9f1d81a19776e47d72bb5ba580f08107c7dc865e1f8c66",
        ),
        ("wasm-tools", "macos", "x86_64") => (
            "1.259.0",
            "bytecodealliance/wasm-tools",
            "wasm-tools-1.259.0-x86_64-macos.tar.gz",
            "7ffdbb9f00207bd16e0612931c63ac1012255a0c805241b339dfe0cf39356a25",
        ),
        ("wasm-tools", "linux", "aarch64") => (
            "1.259.0",
            "bytecodealliance/wasm-tools",
            "wasm-tools-1.259.0-aarch64-linux.tar.gz",
            "9e1644f3841f2b783dd8af82de054667388e46689806fba4909ed45f3182800f",
        ),
        ("wasm-tools", "linux", "x86_64") => (
            "1.259.0",
            "bytecodealliance/wasm-tools",
            "wasm-tools-1.259.0-x86_64-linux.tar.gz",
            "3e9b374b4c7715b771b69bf0d65a337990ed4546ec5e97e01c0ff587dfc52160",
        ),
        ("wasm-tools", "windows", "aarch64") => (
            "1.259.0",
            "bytecodealliance/wasm-tools",
            "wasm-tools-1.259.0-aarch64-windows.zip",
            "32f90b70a645a239171bc9e2f84e6605b5a83723e042c2da477cc83d17263502",
        ),
        ("wasm-tools", "windows", "x86_64") => (
            "1.259.0",
            "bytecodealliance/wasm-tools",
            "wasm-tools-1.259.0-x86_64-windows.zip",
            "3c669d3706a5db7611a11afc422124df53a2c67294575e21f28c9b3deb95fa08",
        ),
        ("wit-bindgen", "macos", "aarch64") => (
            "0.62.0",
            "bytecodealliance/wit-bindgen",
            "wit-bindgen-0.62.0-aarch64-macos.tar.gz",
            "68a8898f8d139d24bd129c5206007dfb7636898edf7659348760d8159ecea9d6",
        ),
        ("wit-bindgen", "macos", "x86_64") => (
            "0.62.0",
            "bytecodealliance/wit-bindgen",
            "wit-bindgen-0.62.0-x86_64-macos.tar.gz",
            "0fe161319d31be62e2a39c8786772230a36e120be457d67c34b67706de91911b",
        ),
        ("wit-bindgen", "linux", "aarch64") => (
            "0.62.0",
            "bytecodealliance/wit-bindgen",
            "wit-bindgen-0.62.0-aarch64-linux.tar.gz",
            "16829d8e4b81ef381c7007ad94d2b14c365360d0024cdfb5a34e5fbd4d8f1d89",
        ),
        ("wit-bindgen", "linux", "x86_64") => (
            "0.62.0",
            "bytecodealliance/wit-bindgen",
            "wit-bindgen-0.62.0-x86_64-linux.tar.gz",
            "3e81cc6523729f7532b4aa7968648a04abf0c711b7d1677150e9121f4e6458fe",
        ),
        ("wit-bindgen", "windows", "aarch64") => (
            "0.62.0",
            "bytecodealliance/wit-bindgen",
            "wit-bindgen-0.62.0-aarch64-windows.zip",
            "c9cccf5ecbf51f0e7dd12d38654c9dbe8aef433711d3c52ffdbb82c22e8a1209",
        ),
        ("wit-bindgen", "windows", "x86_64") => (
            "0.62.0",
            "bytecodealliance/wit-bindgen",
            "wit-bindgen-0.62.0-x86_64-windows.zip",
            "9c9ffe30e8043cf162df53c5d98818e157f9f985d8248a0c2596812b50f0f183",
        ),
        ("componentize-go", "macos", "aarch64") => (
            "0.4.3",
            "bytecodealliance/componentize-go",
            "componentize-go-darwin-arm64.tar.gz",
            "340c26062eb5079fe32fbc325baf5421df7b923de86ef110b8592ee881e4a6cf",
        ),
        ("componentize-go", "macos", "x86_64") => (
            "0.4.3",
            "bytecodealliance/componentize-go",
            "componentize-go-darwin-amd64.tar.gz",
            "0f562d5a98355c6988ea4485e87ae6c08d87fe7423f3ee9252ea8ee2d9d3f77f",
        ),
        ("componentize-go", "linux", "aarch64") => (
            "0.4.3",
            "bytecodealliance/componentize-go",
            "componentize-go-linux-arm64.tar.gz",
            "442b528560bdbe47e4894c228d658d3d4ecc7b4f42516dff7f41cf7770732d82",
        ),
        ("componentize-go", "linux", "x86_64") => (
            "0.4.3",
            "bytecodealliance/componentize-go",
            "componentize-go-linux-amd64.tar.gz",
            "1061d845f550df5d9477612a7d458a31b1e2b8bdc95823704e42e5064deb0c52",
        ),
        ("componentize-go", "windows", "x86_64") => (
            "0.4.3",
            "bytecodealliance/componentize-go",
            "componentize-go-windows-amd64.zip",
            "b7fc1dbfdb9edb97d06d94124b845363bab14ff6a819b518dd6329d0a9428567",
        ),
        ("wasi-sdk", "macos", "aarch64") => (
            "34.0",
            "WebAssembly/wasi-sdk",
            "wasi-sdk-34.0-arm64-macos.tar.gz",
            "9c59398106b417f8f14913380fdf0097a8cc0ff4af9eb3ce0065a859e88d49e9",
        ),
        ("wasi-sdk", "macos", "x86_64") => (
            "34.0",
            "WebAssembly/wasi-sdk",
            "wasi-sdk-34.0-x86_64-macos.tar.gz",
            "87d27fa8adc68dee59bfbf2e22a6d34ef717c34d6bf1d8af2a56fc929d9ce0eb",
        ),
        ("wasi-sdk", "linux", "aarch64") => (
            "34.0",
            "WebAssembly/wasi-sdk",
            "wasi-sdk-34.0-arm64-linux.tar.gz",
            "f7e243dff54d60bcc576e94d6166b69f410f2500ae4a9ceef34315be10e77971",
        ),
        ("wasi-sdk", "linux", "x86_64") => (
            "34.0",
            "WebAssembly/wasi-sdk",
            "wasi-sdk-34.0-x86_64-linux.tar.gz",
            "b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4",
        ),
        ("wasi-sdk", "windows", "aarch64") => (
            "34.0",
            "WebAssembly/wasi-sdk",
            "wasi-sdk-34.0-arm64-windows.tar.gz",
            "45e1c71f3e965621e7b98ebe1d37b0e4b1f77f3e8072113ffb4534e67b1a4b7c",
        ),
        ("wasi-sdk", "windows", "x86_64") => (
            "34.0",
            "WebAssembly/wasi-sdk",
            "wasi-sdk-34.0-x86_64-windows.tar.gz",
            "cccb5c323a9b34f0349a9b09e8804a0a7632c68c3310f4b5f437ed57d7e71d8f",
        ),
        _ => bail!("{name} is not available on {os}/{arch}"),
    };
    let tag = if name == "wasi-sdk" {
        "wasi-sdk-34".to_string()
    } else if name == "uv" {
        version.to_string()
    } else {
        format!("v{version}")
    };
    Ok(Release {
        url: format!("https://github.com/{repository}/releases/download/{tag}/{asset}"),
        sha256,
    })
}

fn download(release: &Release, destination: &Path) -> anyhow::Result<()> {
    eprintln!("Downloading component tool: {}", release.url);
    let mut response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?
        .get(&release.url)
        .send()?
        .error_for_status()?;
    let mut file = File::create(destination)?;
    io::copy(&mut response, &mut file)?;
    drop(file);
    let mut file = File::open(destination)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    if format!("{:x}", hasher.finalize()) != release.sha256 {
        bail!("checksum mismatch for {}", release.url);
    }
    Ok(())
}

fn extract_binary(
    archive: &Path,
    name: &str,
    destination: &Path,
    zip_format: bool,
) -> anyhow::Result<()> {
    let wanted = binary_name(name);
    let output = destination.join(&wanted);
    if zip_format {
        let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
        let mut found = false;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if Path::new(entry.name())
                .file_name()
                .is_some_and(|part| part == std::ffi::OsStr::new(&wanted))
            {
                io::copy(&mut entry, &mut File::create(&output)?)?;
                found = true;
                break;
            }
        }
        if !found {
            bail!("archive does not contain {wanted}");
        }
    } else {
        let decoder = flate2::read::GzDecoder::new(File::open(archive)?);
        let mut archive = tar::Archive::new(decoder);
        let mut found = false;
        for item in archive.entries()? {
            let mut entry = item?;
            if entry.header().entry_type().is_file()
                && entry
                    .path()?
                    .file_name()
                    .is_some_and(|part| part == std::ffi::OsStr::new(&wanted))
            {
                io::copy(&mut entry, &mut File::create(&output)?)?;
                found = true;
                break;
            }
        }
        if !found {
            bail!("archive does not contain {wanted}");
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

pub fn binary(name: &str, version: &str) -> anyhow::Result<PathBuf> {
    let root = cache()?;
    let directory = root.join(format!("{name}-{version}"));
    let binary = directory.join(binary_name(name));
    if binary.is_file() {
        return Ok(binary);
    }
    fs::create_dir_all(&root)?;
    let temporary = tempfile::tempdir_in(&root)?;
    let release = release(name)?;
    let archive = temporary.path().join("download");
    download(&release, &archive)?;
    extract_binary(
        &archive,
        name,
        temporary.path(),
        release.url.ends_with(".zip"),
    )?;
    match fs::rename(temporary.path(), &directory) {
        Ok(()) => Ok(binary),
        Err(_) if binary.is_file() => Ok(binary),
        Err(error) => Err(error.into()),
    }
}

pub fn python_command() -> anyhow::Result<Command> {
    let uv = binary("uv", UV_VERSION)?;
    let mut command = Command::new(uv);
    command.env("UV_CACHE_DIR", cache()?.join("uv-cache"));
    command.args([
        "tool",
        "run",
        "--from",
        &format!("componentize-py=={PY_VERSION}"),
        "componentize-py",
    ]);
    Ok(command)
}

pub fn javascript_command() -> anyhow::Result<Command> {
    let root = cache()?.join(format!(
        "jco-{JCO_VERSION}-componentize-js-{COMPONENTIZE_JS_VERSION}"
    ));
    let executable =
        root.join("node_modules/.bin")
            .join(if cfg!(windows) { "jco.cmd" } else { "jco" });
    if !executable.is_file() {
        fs::create_dir_all(&root)?;
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let output = Command::new(npm)
            .args(["install", "--prefix"])
            .arg(&root)
            .args([
                "--no-audit",
                "--no-fund",
                &format!("@bytecodealliance/jco@{JCO_VERSION}"),
                &format!("@bytecodealliance/componentize-js@{COMPONENTIZE_JS_VERSION}"),
            ])
            .output()
            .context("could not run npm; install Node.js to build JavaScript components")?;
        if !output.status.success() {
            bail!(
                "could not install JavaScript component tools: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(Command::new(executable))
}

pub fn wasi_sdk() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("WASI_SDK_PATH") {
        return Ok(PathBuf::from(path));
    }
    let root = cache()?;
    let directory = root.join("wasi-sdk-34.0");
    if directory
        .join("bin")
        .join(binary_name("wasm32-wasip2-clang"))
        .is_file()
    {
        return Ok(directory);
    }
    fs::create_dir_all(&root)?;
    let temporary = tempfile::tempdir_in(&root)?;
    let archive = temporary.path().join("download.tar.gz");
    download(&release("wasi-sdk")?, &archive)?;
    let unpack = temporary.path().join("unpack");
    fs::create_dir(&unpack)?;
    tar::Archive::new(flate2::read::GzDecoder::new(File::open(&archive)?)).unpack(&unpack)?;
    let extracted = fs::read_dir(&unpack)?
        .next()
        .context("WASI SDK archive is empty")??
        .path();
    match fs::rename(&extracted, &directory) {
        Ok(()) => Ok(directory),
        Err(_) if directory.is_dir() => Ok(directory),
        Err(error) => Err(error.into()),
    }
}

pub fn ensure_rust_target() -> anyhow::Result<()> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("could not run rustup; install the Rust toolchain to build Rust components")?;
    if !output.status.success() {
        bail!("could not inspect installed Rust targets");
    }
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line == "wasm32-unknown-unknown")
    {
        return Ok(());
    }
    eprintln!("Installing Rust target wasm32-unknown-unknown");
    let output = Command::new("rustup")
        .args(["target", "add", "wasm32-unknown-unknown"])
        .output()?;
    if !output.status.success() {
        bail!(
            "could not install Rust target: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
