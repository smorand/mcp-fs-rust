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

/// Rounds of the RAR 3 key derivation (`crypt3.cpp`, `HashRounds`).
const RAR3_ROUNDS: u32 = 0x40000;
/// Every this many rounds one IV byte is taken, 16 in all.
const RAR3_IV_STEP: u32 = RAR3_ROUNDS / 16;
const SHA1_INIT: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

/// A SHA-1 context shaped like UnRAR's (`sha1_context`: state, byte count, one block of
/// buffered input), because the RAR 3 derivation needs to know which blocks are read
/// straight from the caller's buffer. The state update is `sha1::compress`.
#[derive(Clone)]
struct Sha1Rar {
    state: [u32; 5],
    count: u64,
    buffer: [u8; 64],
}

impl Sha1Rar {
    fn new() -> Self {
        Self { state: SHA1_INIT, count: 0, buffer: [0; 64] }
    }

    /// UnRAR's `sha1_process`, or with `write_back` its `sha1_process_rar29`
    /// (`sha1.cpp:146-168`): each whole block hashed straight from `data` (not through
    /// the buffer) is then overwritten with the last 16 words of its message schedule,
    /// little endian. RAR 3 archives were made with that quirk, so a password long enough
    /// to reach it derives another key than plain SHA-1 would. Time O(len), space O(1).
    fn update(&mut self, data: &mut [u8], write_back: bool) {
        let used = (self.count % 64) as usize;
        self.count += data.len() as u64;
        let mut i = 0;
        let mut used_after = used;
        if used + data.len() > 63 {
            i = 64 - used;
            self.buffer[used..].copy_from_slice(&data[..i]);
            sha1::compress(&mut self.state, &[self.buffer.into()]);
            while i + 64 <= data.len() {
                let mut block = [0u8; 64];
                block.copy_from_slice(&data[i..i + 64]);
                sha1::compress(&mut self.state, &[block.into()]);
                if write_back {
                    let words = data[i..i + 64].as_chunks_mut::<4>().0;
                    for (out, word) in words.iter_mut().zip(schedule_tail(&block)) {
                        *out = word.to_le_bytes();
                    }
                }
                i += 64;
            }
            used_after = 0;
        }
        let tail = &data[i..];
        self.buffer[used_after..used_after + tail.len()].copy_from_slice(tail);
    }

    /// The digest words of a copy, padded as SHA-1 is (`sha1_done`), so the derivation
    /// can read intermediate digests and go on.
    fn digest(&self) -> [u32; 5] {
        let mut c = self.clone();
        let bits = c.count.wrapping_mul(8);
        let pos = (c.count % 64) as usize;
        // 0x80, zeros up to 56 modulo 64, then the bit length: 9 to 72 bytes.
        let pad = if pos < 56 { 56 - pos } else { 120 - pos };
        let mut tail = [0u8; 72];
        tail[0] = 0x80;
        tail[pad..pad + 8].copy_from_slice(&bits.to_be_bytes());
        c.update(&mut tail[..pad + 8], false);
        c.state
    }
}

/// Words 64 to 79 of the SHA-1 message schedule of `block`, the words UnRAR's transform
/// leaves in its workspace. Time and space O(1).
fn schedule_tail(block: &[u8; 64]) -> [u32; 16] {
    let mut w = [0u32; 80];
    for (t, word) in block.as_chunks::<4>().0.iter().enumerate() {
        w[t] = u32::from_be_bytes(*word);
    }
    for t in 16..80 {
        w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
    }
    let mut tail = [0u32; 16];
    tail.copy_from_slice(&w[64..]);
    tail
}

/// The RAR 3 AES 128 key and IV of one salt (`crypt3.cpp`, `SetKey30`): the password as
/// UTF-16 little endian then the salt, hashed `0x40000` times with UnRAR's write back,
/// each time followed by the 3 byte round number; every `0x4000` rounds the last byte of
/// the running digest is one IV byte, and the key is the first four digest words, each
/// little endian. Time O(0x40000 * (password length + 8)) SHA-1 work, space
/// O(password length).
pub(crate) fn rar3_kdf(password: &str, salt: &[u8; 8]) -> ([u8; 16], [u8; 16]) {
    let mut raw: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    raw.extend_from_slice(salt);
    let mut sha = Sha1Rar::new();
    let mut iv = [0u8; 16];
    for round in 0..RAR3_ROUNDS {
        sha.update(&mut raw, true);
        let [b0, b1, b2, _] = round.to_le_bytes();
        sha.update(&mut [b0, b1, b2], false);
        if round % RAR3_IV_STEP == 0 {
            iv[(round / RAR3_IV_STEP) as usize] = sha.digest()[4] as u8;
        }
    }
    let digest = sha.digest();
    let mut key = [0u8; 16];
    for (out, word) in key.as_chunks_mut::<4>().0.iter_mut().zip(digest) {
        *out = word.to_le_bytes();
    }
    (key, iv)
}

/// Decrypt `buf` in place with AES 128 CBC, no padding, as `aes256_cbc_decrypt` does.
pub(crate) fn aes128_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], buf: &mut [u8]) {
    let _ = cbc::Decryptor::<aes::Aes128>::new(key.into(), iv.into())
        .decrypt_padded_mut::<NoPadding>(buf);
}

/// Decrypt `buf` in place with AES 256 CBC, no padding. `buf.len()` must be a multiple
/// of 16; the walk only passes whole blocks. Were it not, the buffer is left as is and
/// the header CRC the caller checks next fails, which defers to UnRAR.
pub(crate) fn aes256_cbc_decrypt(key: &[u8; 32], iv: &[u8; 16], buf: &mut [u8]) {
    let _ = cbc::Decryptor::<aes::Aes256>::new(key.into(), iv.into())
        .decrypt_padded_mut::<NoPadding>(buf);
}
