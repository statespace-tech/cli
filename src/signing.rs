//! Ed25519 signatures on variants. CI signs with a private key that never
//! leaves it; `ssp proxy` and `ssp controller` run only variants signed by
//! a key they trust.

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// The bytes a variant's signature covers: its image by digest, app, name,
/// and commit. The backend builds the same message.
pub fn message(app: &str, name: &str, image: &str, commit: Option<&str>) -> String {
    format!(
        "statespace-variant-v2\n{app}\n{name}\n{image}\n{}",
        commit.unwrap_or_default()
    )
}

/// A new key pair as base64: `(private key, public key)`.
pub fn generate() -> (String, String) {
    let key = SigningKey::from_bytes(&rand::random());
    (
        STANDARD.encode(key.to_bytes()),
        STANDARD.encode(key.verifying_key().to_bytes()),
    )
}

pub fn private_key(encoded: &str) -> anyhow::Result<SigningKey> {
    let bytes: [u8; 32] = STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .context("the signing key must be 32 bytes of base64, as `ssp keygen` prints")?;
    Ok(SigningKey::from_bytes(&bytes))
}

pub fn public_key(encoded: &str) -> anyhow::Result<VerifyingKey> {
    let bytes: [u8; 32] = STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .with_context(|| format!("{encoded:?} is not a public key: 32 bytes of base64"))?;
    VerifyingKey::from_bytes(&bytes).context("the public key is invalid")
}

/// Sign `message`, returning `(signature, public key)` as base64.
pub fn sign(key: &SigningKey, message: &str) -> (String, String) {
    (
        STANDARD.encode(key.sign(message.as_bytes()).to_bytes()),
        STANDARD.encode(key.verifying_key().to_bytes()),
    )
}

/// Check that one of `trusted` signed `message`.
pub fn verify(trusted: &[VerifyingKey], signature: &str, message: &str) -> anyhow::Result<()> {
    let signature: [u8; 64] = STANDARD
        .decode(signature)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .context("the signature must be 64 bytes of base64")?;
    let signature = Signature::from_bytes(&signature);
    if trusted
        .iter()
        .any(|key| key.verify(message.as_bytes(), &signature).is_ok())
    {
        return Ok(());
    }
    bail!("no trusted key signed this variant")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_keys_verify_what_they_signed() {
        let (private, public) = generate();
        let signed = message("web", "fast", "ghcr.io/acme/web@sha256:aa", Some("1a2b"));
        let (signature, signer) = sign(&private_key(&private).unwrap(), &signed);
        assert_eq!(signer, public);
        let trusted = [public_key(&public).unwrap()];
        assert!(verify(&trusted, &signature, &signed).is_ok());
        let changed = message("web", "fast", "ghcr.io/evil/web@sha256:aa", Some("1a2b"));
        assert!(verify(&trusted, &signature, &changed).is_err());
        let (_, other) = generate();
        assert!(verify(&[public_key(&other).unwrap()], &signature, &signed).is_err());
    }

    #[test]
    fn matches_the_backend_message() {
        assert_eq!(
            message("web", "fast", "ghcr.io/acme/web@sha256:aa", None),
            "statespace-variant-v2\nweb\nfast\nghcr.io/acme/web@sha256:aa\n"
        );
    }
}
