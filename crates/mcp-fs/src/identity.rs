//! Identity: verified RS256 bearer JWT. Port of the C# `Identity/IdentityResolver.cs`.
//!
//! The token is read from the configured header (default
//! `X-Forwarded-Authorization`) and, failing that, the standard `Authorization`
//! header, so both a gateway and a direct client work. Signature, issuer and
//! expiry/nbf are verified against the configured RSA public key; the caller
//! identity comes from the `username_claim` claim (default `email`) and is
//! normalized caseless.

use crate::config::AuthConfig;
use crate::errors::{Result, ToolError};
use crate::util::normalize_identity;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value;

pub struct IdentityResolver {
    key: Option<DecodingKey>,
    header: String,
    issuer: Option<String>,
    audience: Option<String>,
    username_claim: String,
}

impl IdentityResolver {
    /// Build from config, loading the public key from `auth.jwt.public_key_path`.
    /// A missing key path yields a resolver that rejects every token.
    pub fn new(auth: &AuthConfig) -> Self {
        let key = if auth.jwt.public_key_path.is_empty() {
            None
        } else {
            std::fs::read(&auth.jwt.public_key_path)
                .ok()
                .and_then(|pem| DecodingKey::from_rsa_pem(&pem).ok())
        };
        Self {
            key,
            header: auth.jwt.header.clone(),
            issuer: auth.jwt.issuer.clone(),
            audience: auth.jwt.audience.clone(),
            username_claim: auth.jwt.username_claim.clone(),
        }
    }

    /// Build directly from a PEM public key, for tests.
    pub fn from_pem(pem: &[u8], issuer: Option<&str>, username_claim: &str) -> Result<Self> {
        let key = DecodingKey::from_rsa_pem(pem)
            .map_err(|e| ToolError::internal(format!("invalid RSA public key: {e}")))?;
        Ok(Self {
            key: Some(key),
            header: "X-Forwarded-Authorization".to_string(),
            issuer: issuer.map(str::to_string),
            audience: None,
            username_claim: username_claim.to_string(),
        })
    }

    /// The configured primary header name.
    pub fn header_name(&self) -> &str {
        &self.header
    }

    /// Extract the bearer token from the primary header, then `Authorization`.
    /// Also accepts Basic auth where the password is the token (git CLI compat).
    pub fn bearer_from_headers(&self, get: impl Fn(&str) -> Option<String>) -> Option<String> {
        for name in [self.header.as_str(), "Authorization"] {
            if let Some(raw) = get(name) {
                let t = raw.trim();
                if let Some(rest) = t
                    .strip_prefix("Bearer ")
                    .or_else(|| t.strip_prefix("bearer "))
                {
                    return Some(rest.trim().to_string());
                }
                if let Some(b64) = t
                    .strip_prefix("Basic ")
                    .or_else(|| t.strip_prefix("basic "))
                {
                    use base64::Engine;
                    if let Ok(decoded) =
                        base64::engine::general_purpose::STANDARD.decode(b64.trim())
                        && let Ok(s) = String::from_utf8(decoded) {
                            // "user:token" -> the password is the bearer
                            if let Some((_, pass)) = s.split_once(':')
                                && !pass.is_empty() {
                                    return Some(pass.to_string());
                                }
                        }
                }
                // A bare token with no scheme is accepted too.
                if !t.is_empty() && !t.contains(' ') {
                    return Some(t.to_string());
                }
            }
        }
        None
    }

    /// Verify a token and return the normalized caller identity.
    pub fn verify(&self, token: &str) -> Result<String> {
        let key = self
            .key
            .as_ref()
            .ok_or_else(|| ToolError::unauthenticated("no JWT public key configured"))?;

        let mut v = Validation::new(Algorithm::RS256);
        v.validate_exp = true;
        v.validate_nbf = true;
        // Match the C# TokenValidationParameters.ClockSkew of 30 seconds.
        v.leeway = 30;
        match &self.issuer {
            Some(iss) => v.set_issuer(&[iss]),
            None => v.iss = None,
        }
        match &self.audience {
            Some(aud) => v.set_audience(&[aud]),
            // The C# resolver does not require an audience unless configured.
            None => v.validate_aud = false,
        }

        let data = jsonwebtoken::decode::<Value>(token, key, &v)
            .map_err(|e| ToolError::unauthenticated(format!("invalid token: {e}")))?;

        let person = data
            .claims
            .get(&self.username_claim)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolError::unauthenticated(format!(
                    "token has no '{}' claim",
                    self.username_claim
                ))
            })?;
        if person.trim().is_empty() {
            return Err(ToolError::unauthenticated("token identity claim is empty"));
        }
        Ok(normalize_identity(person))
    }

    /// Header extraction + verification in one step.
    pub fn resolve(&self, get: impl Fn(&str) -> Option<String>) -> Result<String> {
        let token = self
            .bearer_from_headers(get)
            .ok_or_else(|| ToolError::unauthenticated("no bearer token in request headers"))?;
        self.verify(&token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::json;

    /// A throwaway RSA keypair as PEM (private, public).
    fn keypair() -> (Vec<u8>, Vec<u8>) {
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::pkcs8::EncodePublicKey;
        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let pub_pem = pub_key.to_public_key_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        (priv_pem.as_bytes().to_vec(), pub_pem.into_bytes())
    }

    fn mint(priv_pem: &[u8], claims: Value) -> String {
        let key = EncodingKey::from_rsa_pem(priv_pem).unwrap();
        encode(&Header::new(Algorithm::RS256), &claims, &key).unwrap()
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    #[test]
    fn verifies_a_valid_token_and_normalizes_identity() {
        let (pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, Some("web-a2a"), "email").unwrap();
        let t = mint(
            &pk,
            json!({"email": "Alice@Test.COM", "iss": "web-a2a", "exp": now() + 3600, "nbf": now() - 10}),
        );
        assert_eq!(r.verify(&t).unwrap(), "alice@test.com");
    }

    #[test]
    fn rejects_expired_token() {
        let (pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, Some("web-a2a"), "email").unwrap();
        let t = mint(
            &pk,
            json!({"email": "a@b.c", "iss": "web-a2a", "exp": now() - 60, "nbf": now() - 120}),
        );
        let e = r.verify(&t).unwrap_err();
        assert_eq!(e.code, crate::errors::code::UNAUTHENTICATED);
    }

    #[test]
    fn rejects_wrong_issuer() {
        let (pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, Some("web-a2a"), "email").unwrap();
        let t = mint(&pk, json!({"email": "a@b.c", "iss": "someone-else", "exp": now() + 60}));
        assert!(r.verify(&t).is_err());
    }

    #[test]
    fn rejects_token_signed_by_another_key() {
        let (_pk1, pubk1) = keypair();
        let (pk2, _pubk2) = keypair();
        let r = IdentityResolver::from_pem(&pubk1, Some("web-a2a"), "email").unwrap();
        let t = mint(&pk2, json!({"email": "a@b.c", "iss": "web-a2a", "exp": now() + 60}));
        assert!(r.verify(&t).is_err());
    }

    #[test]
    fn rejects_missing_identity_claim() {
        let (pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, Some("web-a2a"), "email").unwrap();
        let t = mint(&pk, json!({"sub": "no-email-here", "iss": "web-a2a", "exp": now() + 60}));
        let e = r.verify(&t).unwrap_err();
        assert!(e.message.contains("no 'email' claim"));
    }

    #[test]
    fn rejects_garbage_token() {
        let (_pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, None, "email").unwrap();
        assert!(r.verify("not.a.jwt").is_err());
        assert!(r.verify("").is_err());
    }

    #[test]
    fn extracts_bearer_from_primary_then_standard_header() {
        let (_pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, None, "email").unwrap();

        let get_primary =
            |n: &str| (n == "X-Forwarded-Authorization").then(|| "Bearer TOK1".to_string());
        assert_eq!(r.bearer_from_headers(get_primary).as_deref(), Some("TOK1"));

        let get_std = |n: &str| (n == "Authorization").then(|| "Bearer TOK2".to_string());
        assert_eq!(r.bearer_from_headers(get_std).as_deref(), Some("TOK2"));

        // primary wins when both are present
        let both = |n: &str| match n {
            "X-Forwarded-Authorization" => Some("Bearer FIRST".to_string()),
            "Authorization" => Some("Bearer SECOND".to_string()),
            _ => None,
        };
        assert_eq!(r.bearer_from_headers(both).as_deref(), Some("FIRST"));
    }

    #[test]
    fn extracts_token_from_basic_auth_password() {
        use base64::Engine;
        let (_pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, None, "email").unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode("git:THETOKEN");
        let get = |n: &str| (n == "Authorization").then(|| format!("Basic {b64}"));
        assert_eq!(r.bearer_from_headers(get).as_deref(), Some("THETOKEN"));
    }

    #[test]
    fn no_headers_means_no_token() {
        let (_pk, pubk) = keypair();
        let r = IdentityResolver::from_pem(&pubk, None, "email").unwrap();
        assert!(r.bearer_from_headers(|_| None).is_none());
        let e = r.resolve(|_| None).unwrap_err();
        assert_eq!(e.code, crate::errors::code::UNAUTHENTICATED);
    }

    #[test]
    fn resolver_without_key_rejects_everything() {
        let auth = AuthConfig::default(); // empty public_key_path
        let r = IdentityResolver::new(&auth);
        assert!(r.verify("anything").is_err());
    }
}
