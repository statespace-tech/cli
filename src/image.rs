//! Image references. A variant names its image by digest, such as
//! `ghcr.io/acme/web@sha256:…`, so its signature covers exact bytes.

use std::process::Command;

use anyhow::{Context, bail};
use serde_json::Value;

/// Whether `image` names a repository and a digest.
pub fn is_digest(image: &str) -> bool {
    image
        .split_once("@sha256:")
        .is_some_and(|(repository, digest)| {
            !repository.is_empty()
                && repository.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
                })
                && digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

/// The repository of a reference: everything before its tag or digest.
pub fn repository(image: &str) -> &str {
    let without_digest = image.split('@').next().unwrap_or(image);
    match without_digest.rsplit_once(':') {
        // A colon after the last slash starts a tag; before it, a registry port.
        Some((repository, tag)) if !tag.contains('/') => repository,
        _ => without_digest,
    }
}

/// Resolve `image` to `repository@sha256:…`. A digest is returned as is. A
/// tag is resolved with `docker`, pulling the image when it is not local.
pub fn resolve(image: &str) -> anyhow::Result<String> {
    if image.contains('@') {
        if is_digest(image) {
            return Ok(image.to_owned());
        }
        bail!("{image} is not a repository and a sha256 digest");
    }
    let digests = match repo_digests(image) {
        Ok(digests) => digests,
        Err(_) => {
            let pulled = Command::new("docker")
                .args(["pull", "--quiet", image])
                .output()
                .context("pass --image by digest, such as ghcr.io/acme/web@sha256:<digest>, or install docker to resolve tags")?;
            if !pulled.status.success() {
                bail!(
                    "could not pull {image}: {}",
                    String::from_utf8_lossy(&pulled.stderr).trim()
                );
            }
            repo_digests(image)?
        }
    };
    pick(image, &digests)
}

/// The digests Docker knows for a local image.
fn repo_digests(image: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .RepoDigests}}",
            image,
        ])
        .output()
        .context("could not run docker")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let digests: Value = serde_json::from_slice(&output.stdout)?;
    Ok(digests
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|digest| Some(digest.as_str()?.to_owned()))
        .collect())
}

/// The digest of `image`'s repository among those Docker reports.
fn pick(image: &str, digests: &[String]) -> anyhow::Result<String> {
    let wanted = normalize(repository(image));
    digests
        .iter()
        .find(|digest| normalize(repository(digest)) == wanted)
        .filter(|digest| is_digest(digest))
        .cloned()
        .with_context(|| {
            format!("{image} has no digest in its registry; push it first, then publish it")
        })
}

/// Docker Hub names a repository several ways.
fn normalize(repository: &str) -> String {
    let repository = repository
        .strip_prefix("docker.io/")
        .or_else(|| repository.strip_prefix("index.docker.io/"))
        .unwrap_or(repository);
    repository
        .strip_prefix("library/")
        .unwrap_or(repository)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_digests() {
        let digest = format!("ghcr.io/acme/web@sha256:{}", "a".repeat(64));
        assert!(is_digest(&digest));
        assert!(is_digest(&format!(
            "localhost:5000/web@sha256:{}",
            "0".repeat(64)
        )));
        assert!(!is_digest("ghcr.io/acme/web:main"));
        assert!(!is_digest(&format!("web@sha256:{}", "A".repeat(64))));
        assert_eq!(resolve(&digest).unwrap(), digest);
        assert!(resolve("web@sha256:abc").is_err());
    }

    #[test]
    fn finds_the_repository_of_a_reference() {
        assert_eq!(repository("ghcr.io/acme/web:main"), "ghcr.io/acme/web");
        assert_eq!(repository("localhost:5000/web"), "localhost:5000/web");
        assert_eq!(repository("localhost:5000/web:v2"), "localhost:5000/web");
        assert_eq!(repository("web@sha256:abc"), "web");
    }

    #[test]
    fn picks_the_digest_of_the_tagged_repository() {
        let ours = format!("ghcr.io/acme/web@sha256:{}", "b".repeat(64));
        let other = format!("registry.example/web@sha256:{}", "c".repeat(64));
        let digests = [other, ours.clone()];
        assert_eq!(pick("ghcr.io/acme/web:main", &digests).unwrap(), ours);
        assert!(pick("ghcr.io/acme/api:main", &digests).is_err());
        let hub = format!("nginx@sha256:{}", "d".repeat(64));
        assert_eq!(
            pick("docker.io/library/nginx:1.27", std::slice::from_ref(&hub)).unwrap(),
            hub
        );
    }
}
