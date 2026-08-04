//! Developer key helpers: generate an RS256 keypair and mint a signed bearer token.
//!
//! Port of the C# `DevCommands.cs`. Why this lives in the server binary at all:
//! a machine without openssl (typically Windows) still needs to bootstrap a
//! local dev identity, so the single executable owns the whole flow.
//!
//! Deviation from C#, deliberate: the private key is written as PKCS#1
//! (`BEGIN RSA PRIVATE KEY`) rather than PKCS#8, because `jsonwebtoken` reads
//! both while the `rsa` crate's PKCS#1 encoder needs no extra feature. The
//! public key stays SPKI (`BEGIN PUBLIC KEY`), which is what
//! `IdentityResolver` loads, so the on-disk `jwt.pub` is byte compatible with
//! the C# `ExportSubjectPublicKeyInfoPem`.

use crate::errors::{Result, ToolError};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::pkcs8::{EncodePublicKey, LineEnding};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// Directory used when `--dir` is omitted, matching the C# default.
pub const DEFAULT_KEY_DIR: &str = ".keys";
/// Private key file name, matching the C# default.
pub const PRIVATE_KEY_FILE: &str = "jwt.key";
/// Public key file name, matching the C# default.
pub const PUBLIC_KEY_FILE: &str = "jwt.pub";
/// Issuer minted into dev tokens, matching `auth.jwt.issuer`'s default.
pub const DEFAULT_ISSUER: &str = "web-a2a";
/// Identity claim minted into dev tokens, matching `auth.jwt.username_claim`.
pub const DEFAULT_CLAIM: &str = "email";
/// Token lifetime in seconds when `--ttl` is omitted.
pub const DEFAULT_TTL_SECONDS: i64 = 3600;
/// RSA modulus size. 2048 is what the C# `RSA.Create(2048)` uses.
const KEY_BITS: usize = 2048;

/// A freshly generated RS256 keypair, both halves PEM encoded.
pub struct Keypair {
    /// PKCS#1 PEM, suitable for `jsonwebtoken::EncodingKey::from_rsa_pem`.
    pub private_pem: String,
    /// SPKI PEM, suitable for `IdentityResolver::from_pem`.
    pub public_pem: String,
}

/// Generate a 2048 bit RSA keypair.
pub fn generate_keypair() -> Result<Keypair> {
    let mut rng = rand::thread_rng();
    let private = rsa::RsaPrivateKey::new(&mut rng, KEY_BITS)
        .map_err(|e| ToolError::internal(format!("RSA key generation failed: {e}")))?;
    let public = private.to_public_key();
    let private_pem = private
        .to_pkcs1_pem(LineEnding::LF)
        .map_err(|e| ToolError::internal(format!("cannot encode private key: {e}")))?
        .to_string();
    let public_pem = public
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| ToolError::internal(format!("cannot encode public key: {e}")))?;
    Ok(Keypair { private_pem, public_pem })
}

/// Generate a keypair and write `jwt.key` / `jwt.pub` into `dir`, creating it if
/// needed. Returns the two paths written, in that order.
pub fn write_keypair(dir: impl AsRef<Path>) -> Result<(PathBuf, PathBuf)> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(|e| {
        ToolError::internal(format!("cannot create key directory {}: {e}", dir.display()))
    })?;
    let kp = generate_keypair()?;
    let key_path = dir.join(PRIVATE_KEY_FILE);
    let pub_path = dir.join(PUBLIC_KEY_FILE);
    // Trailing newline matches the C# writer, so the files diff cleanly across ports.
    std::fs::write(&key_path, format!("{}\n", kp.private_pem.trim_end()))?;
    std::fs::write(&pub_path, format!("{}\n", kp.public_pem.trim_end()))?;
    Ok((key_path, pub_path))
}

/// Mint an RS256 JWT carrying `claim = email`, plus `iss`, `nbf` and `exp`.
///
/// The claim set is exactly the C# `JwtSecurityToken(issuer, claims, notBefore,
/// expires)` output, so a token minted here validates against either server.
pub fn mint_token(
    private_pem: &str,
    email: &str,
    issuer: &str,
    claim: &str,
    ttl_seconds: i64,
) -> Result<String> {
    if email.trim().is_empty() {
        return Err(ToolError::invalid_argument("token identity must not be empty"));
    }
    let key = EncodingKey::from_rsa_pem(private_pem.as_bytes())
        .map_err(|e| ToolError::invalid_argument(format!("invalid RSA private key: {e}")))?;
    let now = chrono::Utc::now().timestamp();
    let mut claims = Map::new();
    claims.insert(claim.to_string(), json!(email));
    claims.insert("nbf".to_string(), json!(now));
    claims.insert("exp".to_string(), json!(now + ttl_seconds));
    claims.insert("iss".to_string(), json!(issuer));
    encode(&Header::new(Algorithm::RS256), &Value::Object(claims), &key)
        .map_err(|e| ToolError::internal(format!("cannot sign token: {e}")))
}

/// Read a PEM private key from disk, then mint a token with it.
pub fn mint_token_from_file(
    key_path: impl AsRef<Path>,
    email: &str,
    issuer: &str,
    claim: &str,
    ttl_seconds: i64,
) -> Result<String> {
    let key_path = key_path.as_ref();
    let pem = std::fs::read_to_string(key_path).map_err(|e| {
        ToolError::invalid_argument(format!("cannot read key {}: {e}", key_path.display()))
    })?;
    mint_token(&pem, email, issuer, claim, ttl_seconds)
}

/// The default private key path (`.keys/jwt.key`).
pub fn default_private_key_path() -> PathBuf {
    Path::new(DEFAULT_KEY_DIR).join(PRIVATE_KEY_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityResolver;

    #[test]
    fn generated_keypair_has_the_expected_pem_headers() {
        let kp = generate_keypair().unwrap();
        assert!(kp.private_pem.starts_with("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(kp.public_pem.starts_with("-----BEGIN PUBLIC KEY-----"));
    }

    #[test]
    fn minted_token_round_trips_through_the_identity_resolver() {
        let kp = generate_keypair().unwrap();
        let token =
            mint_token(&kp.private_pem, "Me@Test.COM", DEFAULT_ISSUER, DEFAULT_CLAIM, 3600).unwrap();
        let resolver =
            IdentityResolver::from_pem(kp.public_pem.as_bytes(), Some(DEFAULT_ISSUER), DEFAULT_CLAIM)
                .unwrap();
        // The resolver normalizes the identity caselessly.
        assert_eq!(resolver.verify(&token).unwrap(), "me@test.com");
    }

    #[test]
    fn minted_token_is_rejected_by_a_foreign_public_key() {
        let a = generate_keypair().unwrap();
        let b = generate_keypair().unwrap();
        let token =
            mint_token(&a.private_pem, "me@test.com", DEFAULT_ISSUER, DEFAULT_CLAIM, 3600).unwrap();
        let resolver =
            IdentityResolver::from_pem(b.public_pem.as_bytes(), Some(DEFAULT_ISSUER), DEFAULT_CLAIM)
                .unwrap();
        assert!(resolver.verify(&token).is_err());
    }

    #[test]
    fn negative_ttl_produces_an_expired_token() {
        let kp = generate_keypair().unwrap();
        // Beyond the resolver's 30 second clock skew allowance.
        let token =
            mint_token(&kp.private_pem, "me@test.com", DEFAULT_ISSUER, DEFAULT_CLAIM, -600).unwrap();
        let resolver =
            IdentityResolver::from_pem(kp.public_pem.as_bytes(), Some(DEFAULT_ISSUER), DEFAULT_CLAIM)
                .unwrap();
        assert!(resolver.verify(&token).is_err());
    }

    #[test]
    fn custom_claim_name_is_honoured() {
        let kp = generate_keypair().unwrap();
        let token =
            mint_token(&kp.private_pem, "me@test.com", DEFAULT_ISSUER, "upn", 3600).unwrap();
        let by_upn =
            IdentityResolver::from_pem(kp.public_pem.as_bytes(), Some(DEFAULT_ISSUER), "upn")
                .unwrap();
        assert_eq!(by_upn.verify(&token).unwrap(), "me@test.com");
        let by_email =
            IdentityResolver::from_pem(kp.public_pem.as_bytes(), Some(DEFAULT_ISSUER), "email")
                .unwrap();
        assert!(by_email.verify(&token).is_err());
    }

    #[test]
    fn empty_identity_is_rejected() {
        let kp = generate_keypair().unwrap();
        let e = mint_token(&kp.private_pem, "  ", DEFAULT_ISSUER, DEFAULT_CLAIM, 60).unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[test]
    fn write_keypair_creates_both_files_and_they_are_usable() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("nested").join("keys");
        let (key_path, pub_path) = write_keypair(&dir).unwrap();
        assert_eq!(key_path, dir.join("jwt.key"));
        assert_eq!(pub_path, dir.join("jwt.pub"));

        let token =
            mint_token_from_file(&key_path, "me@test.com", DEFAULT_ISSUER, DEFAULT_CLAIM, 3600)
                .unwrap();
        let pub_pem = std::fs::read(&pub_path).unwrap();
        let resolver =
            IdentityResolver::from_pem(&pub_pem, Some(DEFAULT_ISSUER), DEFAULT_CLAIM).unwrap();
        assert_eq!(resolver.verify(&token).unwrap(), "me@test.com");
    }

    #[test]
    fn missing_key_file_is_an_invalid_argument() {
        let e = mint_token_from_file(
            "/definitely/not/here/jwt.key",
            "me@test.com",
            DEFAULT_ISSUER,
            DEFAULT_CLAIM,
            60,
        )
        .unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[test]
    fn default_private_key_path_matches_csharp() {
        assert_eq!(default_private_key_path(), Path::new(".keys").join("jwt.key"));
    }
}
