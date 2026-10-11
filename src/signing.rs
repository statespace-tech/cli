//! Ed25519 signatures on variants. CI signs with a private key that never
//! leaves it; `ssp host` runs only variants signed by a key it trusts.

use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};

/// The bytes a variant's signature covers: its bundle, app, name, start
/// command, and commit. The backend builds the same message.
pub fn message(
    app: &str,
    name: &str,
    sha256: &str,
    command: Option<&str>,
    commit: Option<&str>,
) -> String {
    format!(
        "statespace-variant-v1\n{app}\n{name}\n{sha256}\n{}\n{}",
        command.unwrap_or_default(),
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
        .context("the signing key must be 32 bytes of base64, as `ssp variant keygen` prints")?;
    Ok(SigningKey::from_bytes(&bytes))
}

/// Sign `message`, returning `(signature, public key)` as base64.
pub fn sign(key: &SigningKey, message: &str) -> (String, String) {
    (
        STANDARD.encode(key.sign(message.as_bytes()).to_bytes()),
        STANDARD.encode(key.verifying_key().to_bytes()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_with_the_generated_key() {
        let (private, public) = generate();
        let signed = message("web", "fast", "abc", None, Some("1a2b"));
        let (signature, signer) = sign(&private_key(&private).unwrap(), &signed);
        assert_eq!(signer, public);
        assert_eq!(signature.len(), 88);
    }

    #[test]
    fn matches_the_backend_message() {
        assert_eq!(
            message("web", "fast", "abc", Some("serve {port}"), None),
            "statespace-variant-v1\nweb\nfast\nabc\nserve {port}\n"
        );
    }
}
