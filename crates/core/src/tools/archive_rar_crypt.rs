//! Key derivation and block decryption for RAR archives whose headers are encrypted
//! (SPEC-0087 DEC-003, DEC-004), so the completeness walk can read past the encryption
//! header with the right password. Only the RustCrypto crates the tree already compiles
//! are used; nothing here is hand written cryptography. No value computed here is ever
//! logged or put in an error text: the callers only compare and decrypt with it.

use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use hmac::{Hmac, KeyInit, Mac};
use sha2_v011::Sha256;

/// Extra PBKDF2 iterations UnRAR runs after the key for the hash key value, then again
/// for the password check value (`crypt5.cpp`, `pbkdf2`).
const RAR5_EXTRA_ROUNDS: u32 = 16;

/// The RAR5 key and password check value: PBKDF2 HMAC SHA-256 over the UTF-8 password,
/// one 32 byte block, `2^lg2_count` iterations for the key, 16 more for the hash key
/// value (not kept: the walk checks no data checksum) and 16 more for the check value,
/// folded to 8 bytes as UnRAR's `SetKey50` does. The three values come from one loop,
/// as in UnRAR, instead of three PBKDF2 runs. The caller bounds `lg2_count` (at most 24).
/// Time O(2^lg2_count) HMAC computations, space O(1).
pub(crate) fn rar5_kdf(password: &str, salt: &[u8; 16], lg2_count: u8) -> ([u8; 32], [u8; 8]) {
    // An HMAC key of any length is accepted, so this never fails; the keyed state is
    // cloned per iteration so the password is hashed into the pads once.
    let Ok(keyed) = <Hmac<Sha256> as KeyInit>::new_from_slice(password.as_bytes()) else {
        return ([0; 32], [0; 8]);
    };
    let mut mac = keyed.clone();
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u: [u8; 32] = mac.finalize().into_bytes().into();
    let mut f = u;
    let mut values = [[0u8; 32]; 3];
    let counts = [(1u32 << lg2_count) - 1, RAR5_EXTRA_ROUNDS, RAR5_EXTRA_ROUNDS];
    for (value, count) in values.iter_mut().zip(counts) {
        for _ in 0..count {
            let mut mac = keyed.clone();
            mac.update(&u);
            u = mac.finalize().into_bytes().into();
            f.iter_mut().zip(u).for_each(|(a, b)| *a ^= b);
        }
        *value = f;
    }
    let mut check = [0u8; 8];
    for (i, b) in values[2].iter().enumerate() {
        check[i % 8] ^= b;
    }
    (values[0], check)
}

/// Decrypt `buf` in place with AES 256 CBC, no padding. `buf.len()` must be a multiple
/// of 16; the walk only passes whole blocks. Were it not, the buffer is left as is and
/// the header CRC the caller checks next fails, which defers to UnRAR.
pub(crate) fn aes256_cbc_decrypt(key: &[u8; 32], iv: &[u8; 16], buf: &mut [u8]) {
    let _ = cbc::Decryptor::<aes::Aes256>::new(key.into(), iv.into())
        .decrypt_padded_mut::<NoPadding>(buf);
}
