//! AES-256-GCM authenticated encryption for OAuth tokens at rest.
//!
//! Port of the C# `Git/OAuth/TokenCipher.cs`, same blob layout so a database
//! written by either implementation decrypts with the other:
//!
//! ```text
//! nonce(12) || tag(16) || ciphertext(n)
//! ```
//!
//! The key must decode to exactly 32 bytes (AES-256). GCM gives confidentiality
//! plus integrity, so a tampered ciphertext or a wrong key fails to decrypt
//! instead of yielding garbage.

use crate::errors::{Result, ToolError};
use aes_gcm::aead::AeadInPlace;
use aes_gcm::{Aes256Gcm, KeyInit};
use base64::Engine;
use rand::RngCore;

pub const NONCE_SIZE: usize = 12;
pub const TAG_SIZE: usize = 16;
pub const KEY_SIZE: usize = 32;

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Decode a base64 key from the environment and validate its length.
pub fn decode_key(base64_key: &str) -> Result<[u8; KEY_SIZE]> {
    let raw = b64()
        .decode(base64_key.trim())
        .map_err(|_| ToolError::invalid_argument("MCPFS_TOKEN_KEY must be valid base64."))?;
    if raw.len() != KEY_SIZE {
        return Err(ToolError::invalid_argument(format!(
            "MCPFS_TOKEN_KEY must decode to 32 bytes (got {}). \
             Generate one with: openssl rand -base64 32",
            raw.len()
        )));
    }
    let mut key = [0u8; KEY_SIZE];
    key.copy_from_slice(&raw);
    Ok(key)
}

/// A fresh random 32 byte key, base64 encoded (for docs and setup).
pub fn generate_key_base64() -> String {
    let mut key = [0u8; KEY_SIZE];
    rand::rngs::OsRng.fill_bytes(&mut key);
    b64().encode(key)
}

fn cipher(key: &[u8; KEY_SIZE]) -> Aes256Gcm {
    Aes256Gcm::new(key.into())
}

/// Encrypt a token into the `nonce || tag || ciphertext` blob.
pub fn encrypt(key: &[u8; KEY_SIZE], plaintext: &str) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_SIZE];
    rand::rngs::OsRng.fill_bytes(&mut nonce);

    let mut buffer = plaintext.as_bytes().to_vec();
    let tag = cipher(key)
        .encrypt_in_place_detached(&nonce.into(), b"", &mut buffer)
        .map_err(|_| ToolError::internal("OAuth token encryption failed"))?;

    let mut blob = Vec::with_capacity(NONCE_SIZE + TAG_SIZE + buffer.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&tag);
    blob.extend_from_slice(&buffer);
    Ok(blob)
}

/// Decrypt a blob produced by [`encrypt`]. Fails on a wrong key, a tampered blob
/// or a truncated one.
pub fn decrypt(key: &[u8; KEY_SIZE], blob: &[u8]) -> Result<String> {
    if blob.len() < NONCE_SIZE + TAG_SIZE {
        return Err(ToolError::internal(
            "OAuth token blob is too short / corrupt.",
        ));
    }
    let nonce: [u8; NONCE_SIZE] = blob[..NONCE_SIZE].try_into().expect("checked length");
    let tag: [u8; TAG_SIZE] = blob[NONCE_SIZE..NONCE_SIZE + TAG_SIZE]
        .try_into()
        .expect("checked length");
    let mut buffer = blob[NONCE_SIZE + TAG_SIZE..].to_vec();

    cipher(key)
        .decrypt_in_place_detached(&nonce.into(), b"", &mut buffer, &tag.into())
        .map_err(|_| ToolError::internal("OAuth token decryption failed (wrong key or tampered)"))?;
    String::from_utf8(buffer)
        .map_err(|_| ToolError::internal("decrypted OAuth token is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;

    fn key(byte: u8) -> [u8; KEY_SIZE] {
        [byte; KEY_SIZE]
    }

    #[test]
    fn round_trip() {
        let k = key(7);
        let blob = encrypt(&k, "gho_secretvalue").unwrap();
        assert_eq!(decrypt(&k, &blob).unwrap(), "gho_secretvalue");
    }

    #[test]
    fn blob_layout_is_nonce_tag_ciphertext() {
        let k = key(1);
        let plaintext = "0123456789";
        let blob = encrypt(&k, plaintext).unwrap();
        assert_eq!(
            blob.len(),
            NONCE_SIZE + TAG_SIZE + plaintext.len(),
            "GCM ciphertext is the same length as the plaintext"
        );
        // the token must not appear in clear anywhere in the blob
        assert!(!blob.windows(plaintext.len()).any(|w| w == plaintext.as_bytes()));
    }

    #[test]
    fn nonce_is_random_per_call() {
        let k = key(2);
        let a = encrypt(&k, "same").unwrap();
        let b = encrypt(&k, "same").unwrap();
        assert_ne!(a, b, "a reused nonce would be a GCM catastrophe");
        assert_eq!(decrypt(&k, &a).unwrap(), decrypt(&k, &b).unwrap());
    }

    #[test]
    fn empty_and_unicode_plaintexts_round_trip() {
        let k = key(3);
        assert_eq!(decrypt(&k, &encrypt(&k, "").unwrap()).unwrap(), "");
        let s = "clé-token-éàü";
        assert_eq!(decrypt(&k, &encrypt(&k, s).unwrap()).unwrap(), s);
    }

    #[test]
    fn wrong_key_fails() {
        let blob = encrypt(&key(4), "secret").unwrap();
        let e = decrypt(&key(5), &blob).unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("wrong key or tampered"));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let k = key(6);
        let mut blob = encrypt(&k, "secret").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(decrypt(&k, &blob).is_err(), "GCM must reject a modified ciphertext");
    }

    #[test]
    fn tampered_tag_or_nonce_fails() {
        let k = key(8);
        let original = encrypt(&k, "secret").unwrap();

        let mut tag_flipped = original.clone();
        tag_flipped[NONCE_SIZE] ^= 0x01;
        assert!(decrypt(&k, &tag_flipped).is_err());

        let mut nonce_flipped = original;
        nonce_flipped[0] ^= 0x01;
        assert!(decrypt(&k, &nonce_flipped).is_err());
    }

    #[test]
    fn truncated_blob_fails() {
        let k = key(9);
        let blob = encrypt(&k, "secret").unwrap();
        let e = decrypt(&k, &blob[..NONCE_SIZE + TAG_SIZE - 1]).unwrap_err();
        assert!(e.message.contains("too short"));
        assert!(decrypt(&k, &[]).is_err());
    }

    #[test]
    fn decode_key_requires_32_bytes() {
        let good = b64().encode([0u8; 32]);
        assert_eq!(decode_key(&good).unwrap(), [0u8; 32]);
        // whitespace around the value is tolerated, like the C# Trim()
        assert_eq!(decode_key(&format!("  {good}\n")).unwrap(), [0u8; 32]);

        let short = b64().encode([0u8; 16]);
        let e = decode_key(&short).unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("must decode to 32 bytes (got 16)"));

        let long = b64().encode([0u8; 64]);
        assert!(decode_key(&long).unwrap_err().message.contains("got 64"));
    }

    #[test]
    fn decode_key_rejects_non_base64() {
        let e = decode_key("not base64 !!!").unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("must be valid base64"));
    }

    #[test]
    fn generated_key_is_usable_and_random() {
        let a = generate_key_base64();
        let b = generate_key_base64();
        assert_ne!(a, b);
        let k = decode_key(&a).unwrap();
        assert_eq!(decrypt(&k, &encrypt(&k, "tok").unwrap()).unwrap(), "tok");
    }
}
