//! `.rar` decoding for `fs.extract_archive` (SPEC-0080), compiled only with the opt in
//! `rar` feature: the reader is the `unrar` crate, a wrapper of RARLAB's UnRAR whose
//! source licence is non free (DEC-001).

use crate::errors::{Result, ToolError};
use crate::tools::archive::{
    DecodedEntry, ListedEntry, corrupt, ensure_entry_path_safe, password_required_message,
};

/// Name the archive gets inside its per call temp dir; the extension is what the
/// UnRAR library expects, the name itself never reaches the caller.
const TEMP_NAME: &str = "archive.rar";

/// A RAR archive written to its private temp dir and listed: the entries as the
/// headers declare them, every header refusal already applied (SPEC-0085 DEC-007).
/// The temp dir is removed when this drops, after decoding or on any refusal.
pub(crate) struct RarListing {
    /// Held for its `Drop`: UnRAR opens a path only (SPEC-0080 DEC-003), so the
    /// file is written once and read by both passes.
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    pub(crate) entries: Vec<ListedEntry>,
}

/// List a RAR archive held in `bytes` without decoding one entry, applying the
/// header refusals in SPEC-0080 FR-NEW-015 order (completeness, entry rules, then a
/// missing password). Blocking: the caller runs it inside `spawn_blocking`.
/// Time O(n) over the archive bytes written and the n headers, space O(n).
pub(crate) fn list_rar(bytes: &[u8], password: Option<&str>) -> Result<RarListing> {
    // UnRAR reads a truncated archive as a shorter one or as a CRC error, neither of
    // which says corrupt (FR-NEW-011), so the block layout is checked first.
    ensure_rar_complete(bytes, password)?;
    // The crate only opens a path (DEC-003), so the bytes go to a private temp dir
    // that is removed when `dir` drops, on every return path.
    let dir = tempfile::tempdir()
        .map_err(|e| ToolError::internal(format!("rar temp dir failed: {e}")))?;
    let path = dir.path().join(TEMP_NAME);
    std::fs::write(&path, bytes)
        .map_err(|e| ToolError::internal(format!("rar temp file failed: {e}")))?;

    // Entry rules run over a listing pass first, so a link or an escaping name
    // anywhere in the archive is refused before one byte is decoded (FR-NEW-015).
    // With encrypted file names the listing itself cannot be read without the right
    // password, so the password codes here mean "ask for it", not "corrupt" (FR-NEW-004).
    let list_err = |e: unrar::error::UnrarError| listing_error(e, password);
    let mut any_encrypted = false;
    let mut entries = Vec::new();
    for header in open(&path, password).open_for_listing().map_err(list_err)? {
        let header = header.map_err(list_err)?;
        ensure_entry_allowed(&header)?;
        any_encrypted |= header.is_encrypted();
        entries.push(ListedEntry {
            rel_path: entry_name(&header),
            is_dir: header.is_directory(),
            declared: header.unpacked_size,
        });
    }
    // Decided before one byte is read, and after the entry rules (FR-NEW-015 item 7
    // after 6), so a plain entry is never written next to an encrypted one (FR-NEW-002).
    if any_encrypted && password.is_none() {
        return Err(password_required_message(None));
    }
    Ok(RarListing { _dir: dir, path, entries })
}

/// Decode every entry of a listed RAR archive, in archive order. Blocking. `held`
/// receives the running total of decoded entry bytes, also when decoding fails
/// (SPEC-0085 DEC-004). Time O(unpacked size), space O(same): the admission ran
/// on the declared sizes, so a size lie is stopped by UnRAR's own CRC check.
pub(crate) fn decode_rar_listed(
    listing: &RarListing,
    password: Option<&str>,
    held: &mut u64,
) -> Result<Vec<DecodedEntry>> {
    let path = &listing.path;
    let mut cursor = open(path, password).open_for_processing().map_err(|e| corrupt("rar", e))?;
    let mut entries = Vec::new();
    // Entries are read in archive order, because a solid archive can only be
    // decoded front to back.
    while let Some(header) = cursor.read_header().map_err(|e| corrupt("rar", e))? {
        let entry = header.entry();
        let rel_path = entry_name(entry);
        let declared_size = entry.unpacked_size;
        if entry.is_directory() {
            entries.push(DecodedEntry { rel_path, is_dir: true, declared_size, bytes: Vec::new() });
            cursor = header.skip().map_err(|e| corrupt("rar", e))?;
            continue;
        }
        // UnRAR stops at the declared size, so an entry whose data outgrows it
        // fails its CRC here instead of decoding further.
        // On an encrypted entry UnRAR reports a wrong password as BadPassword (RAR5) or as
        // BadData, a CRC error, (RAR4 has no password check value), so those two codes
        // are the password signal; any other code stays corrupt. A plain entry's read
        // error is the oversize signal. An empty password reaches UnRAR as no password at
        // all, so it answers MissingPassword: still a given password here, hence the wrong
        // password message (FR-NEW-020).
        let encrypted = entry.is_encrypted();
        let (data, next) = header.read().map_err(|e| {
            if !encrypted {
                oversized(&rel_path, declared_size)
            } else if matches!(
                e.code,
                unrar::error::Code::BadPassword
                    | unrar::error::Code::BadData
                    | unrar::error::Code::MissingPassword
            ) {
                password_required_message(password)
            } else {
                corrupt("rar", e)
            }
        })?;
        if data.len() as u64 != declared_size {
            return Err(oversized(&rel_path, declared_size));
        }
        *held += data.len() as u64;
        entries.push(DecodedEntry { rel_path, is_dir: false, declared_size, bytes: data });
        cursor = next;
    }
    Ok(entries)
}

/// The six bytes every RAR marker starts with; the seventh tells the version.
const RAR_MARK: &[u8] = b"Rar!\x1a\x07";
/// The RAR5 marker with its eighth byte, for the tests that build archives by hand.
#[cfg(test)]
const RAR5_SIGNATURE: &[u8] = b"Rar!\x1a\x07\x01\x00";
/// UnRAR's `MAXSFXSIZE`: it looks for the marker this far into the file (less 16 bytes).
const MAX_SFX_SIZE: usize = 0x20_0000;
/// Bytes of the marker UnRAR reads first.
const MARK_LEN: usize = 7;
/// Header type of the RAR5 archive encryption block: every header after it is encrypted,
/// so the walk reads further only with the password.
const HEAD_CRYPT: u64 = 4;
/// Header type of the end of archive block, the last block of a complete volume.
const HEAD_END: u64 = 5;
const HFL_EXTRA: u64 = 0x1;
const HFL_DATA: u64 = 0x2;
/// A vint carries 7 bits per byte, so 10 bytes hold any u64.
const VINT_MAX_BYTES: usize = 10;
const TRUNCATED: &str = "unexpected end of archive";
/// RAR4 block types and flags (the base header is 7 bytes: CRC16, type, flags, size).
const RAR4_BASE_HEADER: usize = 7;
const RAR4_MAIN: u8 = 0x73;
const RAR4_FILE: u8 = 0x74;
const RAR4_SERVICE: u8 = 0x7a;
const RAR4_END: u8 = 0x7b;
/// The block is followed by `ADD_SIZE` bytes of data, declared at header offset 7.
const RAR4_LONG_BLOCK: u16 = 0x8000;
/// File and service blocks: the high 32 bits of the data size sit at header offset 32.
const RAR4_LARGE: u16 = 0x0100;
/// Main header flag: every later header is encrypted.
const RAR4_MAIN_PASSWORD: u16 = 0x0080;

/// What the bytes at one position say they are, as UnRAR's `IsSignature` reads them.
enum Signature {
    None,
    /// RAR 1.4, which UnRAR 7 reads and this walk leaves to it.
    Rar14,
    Rar4,
    Rar5,
    /// A marker version UnRAR refuses.
    Future,
}

fn signature(d: &[u8]) -> Signature {
    if d.first() != Some(&0x52) {
        return Signature::None;
    }
    if d.starts_with(b"RE~^") {
        return Signature::Rar14;
    }
    if d.len() >= MARK_LEN && d[1..6] == RAR_MARK[1..] {
        return match d[6] {
            0 => Signature::Rar4,
            1 => Signature::Rar5,
            2..=4 => Signature::Future,
            _ => Signature::None,
        };
    }
    Signature::None
}

/// Where the blocks start and which format they are; `None` when UnRAR will refuse the
/// input or reads a format this walk leaves to it (RAR 1.4, no marker, a RAR5 marker
/// without its eighth byte).
fn find_marker(bytes: &[u8]) -> Option<(bool, &[u8])> {
    if bytes.len() < MARK_LEN {
        return None;
    }
    let (kind, at) = match signature(&bytes[..MARK_LEN]) {
        Signature::None => {
            // UnRAR searches from byte 7, so an archive behind an SFX stub is read from
            // the same marker here (E2E-047). A marker cut by the window end is not seen.
            let window = &bytes[MARK_LEN..bytes.len().min(MARK_LEN + MAX_SFX_SIZE - 16)];
            let mut found = None;
            for i in 0..window.len() {
                match signature(&window[i..]) {
                    Signature::None => {}
                    // UnRAR keeps looking unless the SFX tag sits at byte 28 of the file.
                    Signature::Rar14
                        if i > 0 && window.len() > 31 && &window[21..25] != b"RSFX" => {}
                    kind => {
                        found = Some((kind, MARK_LEN + i));
                        break;
                    }
                }
            }
            found?
        }
        kind => (kind, 0),
    };
    match kind {
        Signature::Rar4 => Some((false, &bytes[at + MARK_LEN..])),
        Signature::Rar5 if bytes.get(at + MARK_LEN) == Some(&0) => {
            Some((true, &bytes[at + MARK_LEN + 1..]))
        }
        _ => None,
    }
}

/// Refuse a RAR archive that holds a whole marker and is not complete: a block whose
/// header or declared data runs past the end, or an end of the input after a block that
/// is not the end of archive block. Bytes after the end block are ignored. Input UnRAR
/// refuses itself, or that this walk cannot read (RAR 1.4), is left to it. With encrypted
/// headers and no password, or an empty one, the walk stops after the leading part and the
/// password questions are UnRAR's; headers are decrypted with a given password.
/// Time O(n) over the blocks, space O(1), no copies.
pub(super) fn ensure_rar_complete(bytes: &[u8], password: Option<&str>) -> Result<()> {
    match find_marker(bytes) {
        None => Ok(()),
        Some((false, rest)) => walk_rar4(rest, password),
        Some((true, rest)) => walk_rar5(rest, password),
    }
}

/// Walk the RAR4 blocks after the marker. Header CRCs are not checked: UnRAR does.
fn walk_rar4(mut rest: &[u8], password: Option<&str>) -> Result<()> {
    loop {
        let base = rest.get(..RAR4_BASE_HEADER).ok_or_else(|| corrupt("rar", TRUNCATED))?;
        let kind = base[2];
        let flags = u16::from_le_bytes([base[3], base[4]]);
        let size = usize::from(u16::from_le_bytes([base[5], base[6]]));
        if size < RAR4_BASE_HEADER {
            return Err(corrupt("rar", TRUNCATED));
        }
        let header = rest.get(..size).ok_or_else(|| corrupt("rar", TRUNCATED))?;
        rest = &rest[size..];
        if kind == RAR4_END {
            return Ok(());
        }
        if kind == RAR4_MAIN && flags & RAR4_MAIN_PASSWORD != 0 {
            return walk_rar4_encrypted(rest, password);
        }
        rest = skip_rar4_data(header, rest)?;
    }
}

/// Skip the data a RAR4 block declares after its header: `ADD_SIZE` when the block is
/// long, plus the high 32 bits of file and service blocks marked large.
fn skip_rar4_data<'a>(header: &[u8], rest: &'a [u8]) -> Result<&'a [u8]> {
    let (kind, flags) = (header[2], u16::from_le_bytes([header[3], header[4]]));
    let mut data = 0u64;
    if flags & RAR4_LONG_BLOCK != 0 {
        let low = header.get(7..11).ok_or_else(|| corrupt("rar", TRUNCATED))?;
        data = u64::from(u32::from_le_bytes([low[0], low[1], low[2], low[3]]));
        if flags & RAR4_LARGE != 0 && (kind == RAR4_FILE || kind == RAR4_SERVICE) {
            let high = header.get(32..36).ok_or_else(|| corrupt("rar", TRUNCATED))?;
            data |= u64::from(u32::from_le_bytes([high[0], high[1], high[2], high[3]])) << 32;
        }
    }
    let data = usize::try_from(data).map_err(|_| corrupt("rar", TRUNCATED))?;
    rest.get(data..).ok_or_else(|| corrupt("rar", TRUNCATED))
}

/// RAR4 salt before each encrypted header (`SIZE_SALT30`).
const RAR4_SALT: usize = 8;

/// Walk the RAR4 blocks after a main header that declares encrypted headers. Without a
/// password, or with an empty one, the answer is UnRAR's (needed, or incorrect). With one,
/// each block is an 8 byte salt then its header in whole AES 128 CBC blocks, then its
/// data, which the walk skips without decrypting (RAR already padded it). Keys are derived once
/// per distinct salt; RAR writes one salt for the whole archive, so a call pays one
/// derivation. A header whose CRC16 fails after decryption, a wrong password as likely as
/// damage since RAR4 has no check value, is left to UnRAR, so its answer stays today's
/// (BL-0136). Time O(0x40000) SHA-1 rounds per salt plus O(n) over the header bytes;
/// space O(largest header).
fn walk_rar4_encrypted(mut rest: &[u8], password: Option<&str>) -> Result<()> {
    use crate::tools::archive_rar_crypt::{aes128_cbc_decrypt, rar3_kdf};
    let Some(password) = password.filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let truncated = || corrupt("rar", TRUNCATED);
    let mut cached: Option<([u8; RAR4_SALT], [u8; 16], [u8; 16])> = None;
    loop {
        let salt: [u8; RAR4_SALT] =
            rest.get(..RAR4_SALT).and_then(|s| s.try_into().ok()).ok_or_else(truncated)?;
        rest = &rest[RAR4_SALT..];
        let (key, iv) = match cached {
            Some((s, key, iv)) if s == salt => (key, iv),
            _ => {
                let (key, iv) = rar3_kdf(password, &salt);
                cached = Some((salt, key, iv));
                (key, iv)
            }
        };
        // CBC decrypts one block from its IV alone, so the first block gives the header
        // size before the rest of the header is touched.
        let mut first = [0u8; AES_BLOCK];
        first.copy_from_slice(rest.get(..AES_BLOCK).ok_or_else(truncated)?);
        aes128_cbc_decrypt(&key, &iv, &mut first);
        let size = usize::from(u16::from_le_bytes([first[5], first[6]]));
        // At least one block, which is already known to be present.
        let padded = size.div_ceil(AES_BLOCK).max(1) * AES_BLOCK;
        let mut plain = rest.get(..padded).ok_or_else(truncated)?.to_vec();
        rest = &rest[padded..];
        aes128_cbc_decrypt(&key, &iv, &mut plain);
        // CRC16 is the low half of the CRC32 of the header after its CRC field. A size
        // below the base header cannot be a header, so it fails the same way.
        let crc_matches = size >= RAR4_BASE_HEADER && {
            let mut crc = flate2::Crc::new();
            crc.update(&plain[2..size]);
            crc.sum().to_le_bytes()[..2] == plain[..2]
        };
        if !crc_matches {
            return Ok(());
        }
        let header = &plain[..size];
        // As in the clear walk, the end block closes the archive before any data.
        if header[2] == RAR4_END {
            return Ok(());
        }
        rest = skip_rar4_data(header, rest)?;
    }
}

/// Walk the RAR5 blocks after the marker. Linear in the number of blocks, no copies.
fn walk_rar5(mut rest: &[u8], password: Option<&str>) -> Result<()> {
    loop {
        // Header CRC32, not verified here: UnRAR checks it.
        rest = rest.get(4..).ok_or_else(|| corrupt("rar", TRUNCATED))?;
        let header_size = read_vint(&mut rest)?;
        let size = usize::try_from(header_size).map_err(|_| corrupt("rar", TRUNCATED))?;
        let mut header = rest.get(..size).ok_or_else(|| corrupt("rar", TRUNCATED))?;
        rest = &rest[size..];
        let kind = read_vint(&mut header)?;
        let flags = read_vint(&mut header)?;
        if flags & HFL_EXTRA != 0 {
            read_vint(&mut header)?;
        }
        let data_size = if flags & HFL_DATA != 0 { read_vint(&mut header)? } else { 0 };
        let data = usize::try_from(data_size).map_err(|_| corrupt("rar", TRUNCATED))?;
        rest = rest.get(data..).ok_or_else(|| corrupt("rar", TRUNCATED))?;
        if kind == HEAD_END {
            return Ok(());
        }
        if kind == HEAD_CRYPT {
            return walk_rar5_encrypted(header, rest, password);
        }
    }
}

/// UnRAR's `CRYPT5_KDF_LG2_COUNT_MAX`: a larger iteration count is a damaged header.
const RAR5_KDF_LG2_MAX: u8 = 24;
/// Encryption header flag: an 8 byte password check value and its 4 byte sum follow.
const RAR5_CHECK_PRESENT: u64 = 0x1;
const AES_BLOCK: usize = 16;

/// Walk the RAR5 blocks after the encryption header, whose body (after its type and
/// flags) is `crypt`. Without a password, or with an empty one, the answer is UnRAR's
/// (needed, or incorrect). With one, the keys are derived once; a check value that
/// disagrees is the wrong password, whether the rest is cut or not (FR-NEW-005). Each
/// later header is a 16 byte IV then whole AES blocks; a header whose CRC fails after
/// decryption is left to UnRAR, so its answer stays today's. Time O(2^lg2_count) for the
/// derivation plus O(n) over the header bytes; space O(largest header).
fn walk_rar5_encrypted(mut crypt: &[u8], mut rest: &[u8], password: Option<&str>) -> Result<()> {
    use crate::tools::archive_rar_crypt::{aes256_cbc_decrypt, rar5_kdf};
    let Some(password) = password.filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let truncated = || corrupt("rar", TRUNCATED);
    read_vint(&mut crypt)?;
    let flags = read_vint(&mut crypt)?;
    let (&lg2_count, crypt) = crypt.split_first().ok_or_else(truncated)?;
    if lg2_count > RAR5_KDF_LG2_MAX {
        return Err(corrupt("rar", "key derivation count is too large"));
    }
    let salt: &[u8; 16] = crypt.get(..16).and_then(|s| s.try_into().ok()).ok_or_else(truncated)?;
    let (key, derived_check) = rar5_kdf(password, salt, lg2_count);
    if flags & RAR5_CHECK_PRESENT != 0 {
        let check = crypt.get(16..24).ok_or_else(truncated)?;
        let sum = crypt.get(24..28).ok_or_else(truncated)?;
        // A check value whose own sum fails is damaged, so it says nothing about the
        // password, as in UnRAR.
        let valid = {
            use sha2_v011::Digest;
            sha2_v011::Sha256::digest(check)[..4] == *sum
        };
        if valid && check != derived_check {
            return Err(password_required_message(Some(password)));
        }
    }
    loop {
        let iv: &[u8; 16] =
            rest.get(..AES_BLOCK).and_then(|s| s.try_into().ok()).ok_or_else(truncated)?;
        rest = &rest[AES_BLOCK..];
        // CBC decrypts one block from its IV alone, so the first block gives the CRC and
        // the header size before the rest of the header is touched.
        let mut first = [0u8; AES_BLOCK];
        first.copy_from_slice(rest.get(..AES_BLOCK).ok_or_else(truncated)?);
        aes256_cbc_decrypt(&key, iv, &mut first);
        let mut size_field = &first[4..];
        // Undecodable plaintext cannot be told from a wrong key without a check value;
        // UnRAR answers it.
        let Ok(header_size) = read_vint(&mut size_field) else {
            return Ok(());
        };
        let vint_len = AES_BLOCK - 4 - size_field.len();
        let total = usize::try_from(header_size)
            .ok()
            .and_then(|s| s.checked_add(4 + vint_len))
            .ok_or_else(truncated)?;
        let padded = total.div_ceil(AES_BLOCK).checked_mul(AES_BLOCK).ok_or_else(truncated)?;
        let mut plain = rest.get(..padded).ok_or_else(truncated)?.to_vec();
        rest = &rest[padded..];
        aes256_cbc_decrypt(&key, iv, &mut plain);
        let mut crc = flate2::Crc::new();
        crc.update(&plain[4..total]);
        if crc.sum().to_le_bytes() != plain[..4] {
            return Ok(());
        }
        let mut header = &plain[4 + vint_len..total];
        let kind = read_vint(&mut header)?;
        let flags = read_vint(&mut header)?;
        if flags & HFL_EXTRA != 0 {
            read_vint(&mut header)?;
        }
        let data_size = if flags & HFL_DATA != 0 { read_vint(&mut header)? } else { 0 };
        let data = usize::try_from(data_size).map_err(|_| truncated())?;
        rest = rest.get(data..).ok_or_else(truncated)?;
        if kind == HEAD_END {
            return Ok(());
        }
    }
}

/// Read one RAR5 vint from the front of `input`, refusing more than ten bytes or a
/// value past u64 instead of wrapping.
fn read_vint(input: &mut &[u8]) -> Result<u64> {
    let mut value = 0u64;
    for (i, &byte) in input.iter().take(VINT_MAX_BYTES).enumerate() {
        let bits = u64::from(byte & 0x7f);
        let shift = 7 * i as u32;
        // The tenth byte may only hold the single top bit of a u64.
        if shift == 63 && bits > 1 {
            return Err(corrupt("rar", "variable length integer overflows"));
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            *input = &input[i + 1..];
            return Ok(value);
        }
    }
    if input.len() < VINT_MAX_BYTES {
        Err(corrupt("rar", TRUNCATED))
    } else {
        Err(corrupt("rar", "variable length integer is too long"))
    }
}

/// A password code from the listing pass means the names are encrypted and the password
/// is missing or wrong; any other code is a damaged archive. The library's own text is
/// only used for the latter, and never holds the password.
fn listing_error(e: unrar::error::UnrarError, password: Option<&str>) -> ToolError {
    use unrar::error::Code;
    if matches!(e.code, Code::MissingPassword | Code::BadPassword) {
        password_required_message(password)
    } else {
        corrupt("rar", e)
    }
}

fn open<'a>(path: &'a std::path::Path, password: Option<&'a str>) -> unrar::Archive<'a> {
    match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    }
}

/// UnRAR hands back the platform separator; the volume always speaks `/`. The
/// library does not sanitize `..` or a leading `/`, so the raw name is checked.
fn entry_name(entry: &unrar::FileHeader) -> String {
    entry.filename.to_string_lossy().replace('\\', "/")
}

/// Unix mode file type mask and the types RAR stores from a Unix host's `st_mode`
/// in `file_attr`.
const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;
/// Windows `FILE_ATTRIBUTE_REPARSE_POINT`, how a Windows host marks a symlink.
const ATTR_REPARSE_POINT: u32 = 0x400;

/// Refuse a symlink (FR-NEW-006) or an escaping name (FR-NEW-007). A RAR5 hard link
/// looks like an ordinary file through the safe API (`file_attr` and size carry no
/// trace of the redirection record), so it is not refused here (US-0171 Blocked
/// section). Devices are not classified either: RAR never stores them, and the
/// Windows attribute bits 0x1000 and 0x2000 share their values with the Unix
/// FIFO and character device types, so a mode based test would refuse ordinary
/// Windows files.
fn ensure_entry_allowed(entry: &unrar::FileHeader) -> Result<()> {
    let name = entry_name(entry);
    if let Some(kind) = refused_kind(entry.file_attr) {
        return Err(ToolError::not_supported(format!(
            "archive entry '{name}' is a {kind}, which is not supported"
        )));
    }
    ensure_entry_path_safe(&name)
}

/// The refused kind an attribute word names, if any.
fn refused_kind(file_attr: u32) -> Option<&'static str> {
    let kind = file_attr & S_IFMT;
    // 0x400 is also the setgid bit of a Unix mode, so the reparse flag only counts
    // when the type bits do not say regular file or folder.
    let reparse = file_attr & ATTR_REPARSE_POINT != 0 && !matches!(kind, S_IFREG | S_IFDIR);
    (kind == S_IFLNK || reparse).then_some("symlink")
}

fn oversized(rel_path: &str, declared: u64) -> ToolError {
    crate::tools::archive::declared_size_exceeded(rel_path, declared)
}

#[cfg(test)]
mod tests {
    use crate::errors::Result;
    use crate::tools::admin::test_support::Fixture;
    use crate::tools::archive::extract_archive;
    use crate::tools::archive_rar_fixtures::{RarEntry, rar5_store};
    use serde_json::Value;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "proj";

    async fn fixture() -> Fixture {
        let f = Fixture::with_config(|_| {}).await;
        f.seed_project(MOUNT, OWNER).await;
        f.state.stores.client(MOUNT).await.unwrap().makedirs("/uploads", true).await.unwrap();
        f
    }

    async fn seed(f: &Fixture, path: &str, bytes: &[u8]) {
        f.state.stores.client(MOUNT).await.unwrap().write_bytes_atomic(path, bytes).await.unwrap();
    }

    async fn read(f: &Fixture, path: &str) -> Vec<u8> {
        f.state.stores.client(MOUNT).await.unwrap().read_bytes(path).await.unwrap()
    }

    async fn extract(f: &Fixture, path: &str) -> Result<Value> {
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        extract_archive(&f.state, MOUNT, OWNER, path, None, false, None).await
    }

    fn report() -> Vec<u8> {
        rar5_store(&[
            RarEntry::dir("data"),
            RarEntry::file("data/a.txt", b"hello"),
            RarEntry::file("data/b.txt", b"goodbye"),
        ])
    }

    #[test]
    fn attribute_kinds_are_classified() {
        use super::refused_kind;
        assert_eq!(refused_kind(0o120_777), Some("symlink"));
        assert_eq!(refused_kind(0x400), Some("symlink"));
        // Windows attributes: 0x2020 (not indexed, archive) is an ordinary file, 0x2420
        // adds the reparse flag and is a symlink
        assert_eq!(refused_kind(0x2020), None);
        assert_eq!(refused_kind(0x1020), None);
        assert_eq!(refused_kind(0x2420), Some("symlink"));
        // setgid regular file and folder share bit 0x400 with the reparse flag
        assert_eq!(refused_kind(0o102_755), None);
        assert_eq!(refused_kind(0o042_755), None);
        assert_eq!(refused_kind(0o100_644), None);
        assert_eq!(refused_kind(0x20), None);
    }

    /// Bytes that are not a RAR archive surface as the shared corrupt message.
    #[test]
    fn garbage_is_a_corrupt_rar() {
        let e = super::list_rar(b"not a rar archive", None).err().expect("a refusal");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(
            e.message.starts_with("archive is corrupt or not a valid rar file:"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn e2e_0080_001_plain_rar_extracts_to_the_default_de() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        let v = extract(&f, "/uploads/report.rar").await.unwrap();
        assert_eq!(v["destination"], "/uploads/report");
        assert_eq!(v["files_written"], 2);
        assert_eq!(v["dirs_created"], 2);
        assert_eq!(v["bytes_written"], 12);
        assert_eq!(read(&f, "/uploads/report/data/a.txt").await, b"hello");
        assert_eq!(read(&f, "/uploads/report/data/b.txt").await, b"goodbye");
        let log = f.state.safety.audit(OWNER, MOUNT);
        let lines: Vec<_> = log.iter().filter(|e| e.op == "extract_archive").collect();
        assert_eq!(lines.len(), 1, "{log:?}");
        assert_eq!(lines[0].path, "/uploads/report.rar");
    }

    #[tokio::test]
    async fn e2e_0080_002_upper_case_suffix_is_detected() {
        let f = fixture().await;
        seed(&f, "/uploads/REPORT.RAR", &report()).await;
        let v = extract(&f, "/uploads/REPORT.RAR").await.unwrap();
        assert_eq!(v["destination"], "/uploads/REPORT");
        assert_eq!(v["files_written"], 2);
    }

    #[tokio::test]
    async fn e2e_0080_017_empty_rar() {
        let f = fixture().await;
        seed(&f, "/uploads/none.rar", &rar5_store(&[])).await;
        let v = extract(&f, "/uploads/none.rar").await.unwrap();
        assert_eq!(v["files_written"], 0);
        assert_eq!(v["dirs_created"], 1);
        assert_eq!(v["bytes_written"], 0);
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert!(c.exists("/uploads/none").await.unwrap());
        assert!(c.list_dir("/uploads/none").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn e2e_0080_018_accents_and_spaces() {
        let f = fixture().await;
        seed(
            &f,
            "/uploads/noms.rar",
            &rar5_store(&[RarEntry::file("été 2026/compte rendu.txt", b"ok")]),
        )
        .await;
        let v = extract(&f, "/uploads/noms.rar").await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/noms/été 2026/compte rendu.txt").await, b"ok");
    }

    async fn assert_absent(f: &Fixture, paths: &[&str]) {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        for p in paths {
            assert!(!c.exists(p).await.unwrap(), "{p} must not exist");
        }
    }

    async fn refuse(f: &Fixture, archive: &str, bytes: Vec<u8>) -> crate::errors::ToolError {
        seed(f, archive, &bytes).await;
        extract(f, archive).await.unwrap_err()
    }

    #[tokio::test]
    async fn e2e_0080_008_symbolic_link_entry() {
        let f = fixture().await;
        let e = refuse(
            &f,
            "/uploads/links.rar",
            rar5_store(&[RarEntry::file("a.txt", b"x"), RarEntry::symlink("l", "/etc/passwd")]),
        )
        .await;
        assert_eq!(e.code, crate::errors::code::NOT_SUPPORTED);
        assert!(e.message.contains("'l'") && e.message.contains("symlink"), "{}", e.message);
        assert_absent(&f, &["/uploads/links"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_009_entry_named_etc_passwd() {
        let f = fixture().await;
        let e = refuse(
            &f,
            "/uploads/evil.rar",
            rar5_store(&[RarEntry::file("../../etc/passwd", b"pwned")]),
        )
        .await;
        assert_eq!(e.code, crate::errors::code::PATH_OUT_OF_BOUNDS);
        assert_absent(&f, &["/etc/passwd", "/uploads/etc", "/uploads/evil"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_010_collision_names_the_first_colliding() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        seed(&f, "/uploads/report/data/a.txt", b"old a").await;
        seed(&f, "/uploads/report/data/b.txt", b"old b").await;
        let e = extract(&f, "/uploads/report.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
        // The archive lists the folder `data` before its files, and the shared scan
        // counts an existing folder as a collision (e2e_new_049), so the first
        // colliding path in archive order is the folder. See SPEC-0080 erratum 2.
        assert!(e.message.contains("'/uploads/report/data'"), "{}", e.message);
        assert_eq!(read(&f, "/uploads/report/data/a.txt").await, b"old a");
        assert_eq!(read(&f, "/uploads/report/data/b.txt").await, b"old b");
    }

    #[tokio::test]
    async fn e2e_0080_012_entry_that_decodes_larger_than_decla() {
        let f = fixture().await;
        let client = f.state.stores.client(MOUNT).await.unwrap();
        let archive = rar5_store(&[RarEntry::declaring("big.bin", 10, &vec![7u8; 1_000_000])]);
        seed(&f, "/uploads/bomb.rar", &archive).await;
        let used_before = crate::quota::used_bytes(&client).await.unwrap();
        let session_before = f.state.safety.session_remaining(OWNER, MOUNT);
        let e = extract(&f, "/uploads/bomb.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("big.bin"), "{}", e.message);
        assert!(e.message.contains("more bytes than declared 10"), "{}", e.message);
        assert_absent(&f, &["/uploads/bomb"]).await;
        // The refusal charged nothing: storage and session allowance are unchanged.
        assert_eq!(crate::quota::used_bytes(&client).await.unwrap(), used_before);
        assert_eq!(f.state.safety.session_remaining(OWNER, MOUNT), session_before);
    }

    #[tokio::test]
    async fn e2e_0080_019_overwrite_true_replaces() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        seed(&f, "/uploads/report/data/a.txt", b"old").await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let v = extract_archive(&f.state, MOUNT, OWNER, "/uploads/report.rar", None, true, None)
            .await
            .unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(read(&f, "/uploads/report/data/a.txt").await, b"hello");
    }

    /// The reader shows a hard link as an empty file (SPEC-0080 erratum 2), so it
    /// extracts as one: nothing is linked.
    #[tokio::test]
    async fn e2e_0080_032_hard_link_entry() {
        let f = fixture().await;
        seed(
            &f,
            "/uploads/hl.rar",
            &rar5_store(&[RarEntry::file("a.txt", b"x"), RarEntry::hard_link("b.txt", "a.txt")]),
        )
        .await;
        extract(&f, "/uploads/hl.rar").await.unwrap();
        assert_eq!(read(&f, "/uploads/hl/a.txt").await, b"x");
        assert_eq!(read(&f, "/uploads/hl/b.txt").await, b"");
    }

    #[tokio::test]
    async fn e2e_0080_033_link_after_valid_entries_writes_noth() {
        let f = fixture().await;
        let e = refuse(
            &f,
            "/uploads/mix.rar",
            rar5_store(&[
                RarEntry::file("ok1.txt", b"1"),
                RarEntry::file("ok2.txt", b"2"),
                RarEntry::symlink("l", "ok1.txt"),
            ]),
        )
        .await;
        assert_eq!(e.code, crate::errors::code::NOT_SUPPORTED);
        assert_absent(&f, &["/uploads/mix"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_034_absolute_entry_name() {
        let f = fixture().await;
        let e = refuse(
            &f,
            "/uploads/abs.rar",
            rar5_store(&[RarEntry::file("/etc/cron.d/x", b"* * * * * root id")]),
        )
        .await;
        assert_eq!(e.code, crate::errors::code::PATH_OUT_OF_BOUNDS);
        assert_absent(&f, &["/etc/cron.d/x", "/uploads/abs"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_035_traversal_in_the_middle_of_a_name() {
        let f = fixture().await;
        let e =
            refuse(&f, "/uploads/mid.rar", rar5_store(&[RarEntry::file("a/../../b.txt", b"x")]))
                .await;
        assert_eq!(e.code, crate::errors::code::PATH_OUT_OF_BOUNDS);
        assert_absent(&f, &["/uploads/b.txt", "/b.txt", "/uploads/mid"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_036_one_oversized_entry_among_honest_one() {
        let f = fixture().await;
        let e = refuse(
            &f,
            "/uploads/one.rar",
            rar5_store(&[
                RarEntry::file("ok.txt", b"fine"),
                RarEntry::declaring("liar.bin", 10, &[7u8; 11]),
            ]),
        )
        .await;
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("liar.bin"), "{}", e.message);
        assert_absent(&f, &["/uploads/one"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_037_declared_10_bytes_decoded_10_bytes() {
        let f = fixture().await;
        seed(
            &f,
            "/uploads/exact.rar",
            &rar5_store(&[RarEntry::declaring("exact.bin", 10, &[7u8; 10])]),
        )
        .await;
        let v = extract(&f, "/uploads/exact.rar").await.unwrap();
        assert_eq!(v["files_written"], 1);
        assert_eq!(v["bytes_written"], 10);
    }

    #[tokio::test]
    async fn e2e_0080_044_collision_under_an_explicit_destinat() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        seed(&f, "/out/data/b.txt", b"old").await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let e = extract_archive(
            &f.state,
            MOUNT,
            OWNER,
            "/uploads/report.rar",
            Some("/out"),
            false,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);
        // The existing folder `/out/data` is the first collision in archive order.
        assert!(e.message.contains("'/out/data'"), "{}", e.message);
        assert_absent(&f, &["/out/data/a.txt"]).await;
    }

    /// Give the project exactly `n` bytes of remaining storage.
    async fn limit_remaining(f: &Fixture, n: i64) {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let used = crate::quota::used_bytes(&c).await.unwrap();
        f.state.admin.set_quota(MOUNT, Some(used + n)).await.unwrap();
    }

    async fn remaining_storage(f: &Fixture) -> i64 {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let limit = f.state.safety.project_limit(MOUNT).await.unwrap().unwrap();
        limit - crate::quota::used_bytes(&c).await.unwrap()
    }

    fn allowance(f: &Fixture) -> i64 {
        f.state.safety.session_remaining(OWNER, MOUNT)
    }

    /// A fixture whose member may write `quota` bytes in the session.
    async fn fixture_with_allowance(quota: i64) -> Fixture {
        let f = Fixture::with_config(move |c| c.safety.write_quota_bytes = quota).await;
        f.seed_project(MOUNT, OWNER).await;
        f.state.stores.client(MOUNT).await.unwrap().makedirs("/uploads", true).await.unwrap();
        f
    }

    async fn small_write(f: &Fixture) -> Result<Value> {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        crate::core::fs_ops::write_bytes(
            &c,
            &f.state.safety,
            OWNER,
            MOUNT,
            "/uploads/small.txt",
            b"abc",
            false,
            true,
        )
        .await
    }

    fn project_refusal(v: &Result<Value>) {
        let e = v.as_ref().expect_err("the extraction must be refused");
        assert_eq!(e.code, crate::errors::code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
        assert!(e.message.starts_with("project storage quota of"), "{}", e.message);
    }

    fn session_refusal(v: &Result<Value>, quota: i64) {
        let e = v.as_ref().expect_err("the extraction must be refused");
        assert_eq!(e.code, crate::errors::code::WRITE_QUOTA_EXCEEDED, "{e:?}");
        assert_eq!(e.message, format!("session write quota of {quota} bytes exceeded"));
    }

    #[tokio::test]
    async fn e2e_0080_011_over_the_project_storage_limit() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        limit_remaining(&f, 5).await;
        let v = extract(&f, "/uploads/report.rar").await;
        project_refusal(&v);
        assert_eq!(remaining_storage(&f).await, 5);
        assert_absent(&f, &["/uploads/report"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_038_storage_exactly_equal_to_the_declare() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        limit_remaining(&f, 12).await;
        let v = extract(&f, "/uploads/report.rar").await.unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(remaining_storage(&f).await, 0);
    }

    #[tokio::test]
    async fn e2e_0080_041_a_refused_storage_limit_leaves_the_p() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        limit_remaining(&f, 5).await;
        project_refusal(&extract(&f, "/uploads/report.rar").await);
        small_write(&f).await.unwrap();
        assert_eq!(remaining_storage(&f).await, 2);
    }

    #[tokio::test]
    async fn e2e_0080_073_overwrite_counts_net_growth_against() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        seed(&f, "/uploads/report/data/a.txt", b"0123456789").await;
        seed(&f, "/uploads/report/data/b.txt", b"0123456789").await;
        limit_remaining(&f, 5).await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let v = extract_archive(&f.state, MOUNT, OWNER, "/uploads/report.rar", None, true, None)
            .await
            .unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(remaining_storage(&f).await, 13);
    }

    #[tokio::test]
    async fn e2e_0080_077_over_the_session_write_allowance() {
        let f = fixture_with_allowance(5).await;
        seed(&f, "/uploads/report.rar", &report()).await;
        let v = extract(&f, "/uploads/report.rar").await;
        session_refusal(&v, 5);
        assert_absent(&f, &["/uploads/report"]).await;
        assert_eq!(allowance(&f), 5);
    }

    #[tokio::test]
    async fn e2e_0080_078_the_allowance_is_charged_the_full_de() {
        let f = fixture_with_allowance(20).await;
        seed(&f, "/uploads/report.rar", &report()).await;
        seed(&f, "/uploads/report/data/a.txt", b"0123456789").await;
        seed(&f, "/uploads/report/data/b.txt", b"0123456789").await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let v = extract_archive(&f.state, MOUNT, OWNER, "/uploads/report.rar", None, true, None)
            .await
            .unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(allowance(&f), 8);
    }

    #[tokio::test]
    async fn e2e_0080_079_allowance_equal_to_the_declared_tota() {
        let f = fixture_with_allowance(12).await;
        seed(&f, "/uploads/report.rar", &report()).await;
        let v = extract(&f, "/uploads/report.rar").await.unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(allowance(&f), 0);
    }

    #[tokio::test]
    async fn e2e_0080_080_a_refused_extraction_charges_nothing() {
        let f = fixture_with_allowance(5).await;
        seed(&f, "/uploads/report.rar", &report()).await;
        session_refusal(&extract(&f, "/uploads/report.rar").await, 5);
        small_write(&f).await.unwrap();
        assert_eq!(allowance(&f), 2);
    }

    const MULTI_VOLUME_MESSAGE: &str =
        "multi volume RAR archives are not supported: extract a single file archive";

    fn one_file() -> Vec<u8> {
        rar5_store(&[RarEntry::file("a.txt", b"hello")])
    }

    async fn assert_multi_volume_refused(name: &str, stem: &str) {
        let f = fixture().await;
        let e = refuse(&f, name, one_file()).await;
        assert_eq!(e.code, crate::errors::code::NOT_SUPPORTED);
        assert_eq!(e.message, MULTI_VOLUME_MESSAGE);
        assert_absent(&f, &[stem]).await;
    }

    #[tokio::test]
    async fn e2e_0080_013_multi_volume_name() {
        assert_multi_volume_refused("/uploads/big.part1.rar", "/uploads/big.part1").await;
    }

    /// Garbage bytes under a multi volume name prove the refusal comes from the name,
    /// before the file is read: a read would have given the corrupt message.
    #[tokio::test]
    async fn multi_volume_name_is_refused_before_the_file_is_read() {
        let f = fixture().await;
        let e = refuse(&f, "/uploads/big.part1.rar", b"not a rar archive".to_vec()).await;
        assert_eq!(e.code, crate::errors::code::NOT_SUPPORTED);
        assert_eq!(e.message, MULTI_VOLUME_MESSAGE);
    }

    #[tokio::test]
    async fn e2e_0080_014_truncated_rar() {
        let f = fixture().await;
        // The structural pre check refuses any cut; every prefix is covered by the
        // unit tests below.
        let full = one_file();
        let e = refuse(&f, "/uploads/broken.rar", full[..full.len() / 2].to_vec()).await;
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(
            e.message.starts_with("archive is corrupt or not a valid rar file:"),
            "{}",
            e.message
        );
        assert_absent(&f, &["/uploads/broken"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_014_truncated_report_rar_at_half() {
        let f = fixture().await;
        let full = report();
        let e = refuse(&f, "/uploads/half.rar", full[..full.len() / 2].to_vec()).await;
        assert!(
            e.message.starts_with("archive is corrupt or not a valid rar file:"),
            "{}",
            e.message
        );
    }

    #[test]
    fn every_strict_prefix_of_a_rar5_archive_is_corrupt() {
        let full = rar5_store(&[RarEntry::file("a.txt", b"hello"), RarEntry::dir("d")]);
        super::ensure_rar_complete(&full, None).unwrap();
        for len in super::RAR5_SIGNATURE.len()..full.len() {
            let e = super::ensure_rar_complete(&full[..len], None).unwrap_err();
            assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "len {len}");
            assert!(
                e.message.starts_with("archive is corrupt or not a valid rar file:"),
                "len {len}"
            );
        }
    }

    #[test]
    fn partial_markers_are_left_to_the_library() {
        super::ensure_rar_complete(b"", None).unwrap();
        super::ensure_rar_complete(b"Rar!\x1a\x07\x01", None).unwrap();
    }

    #[test]
    fn rar4_marker_alone_is_truncated() {
        let e = super::ensure_rar_complete(b"Rar!\x1a\x07\x00", None).unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert_eq!(
            e.message,
            "archive is corrupt or not a valid rar file: unexpected end of archive"
        );
    }

    #[test]
    fn vint_with_eleven_bytes_is_refused() {
        let mut input = [0x80u8; 11].to_vec();
        input.push(0);
        let e = super::read_vint(&mut &input[..]).unwrap_err();
        assert!(e.message.ends_with("variable length integer is too long"), "{}", e.message);
        let mut archive = super::RAR5_SIGNATURE.to_vec();
        archive.extend([0; 4]);
        archive.extend(&input);
        assert!(super::ensure_rar_complete(&archive, None).is_err());
    }

    #[test]
    fn vint_past_u64_is_refused() {
        let input = [0xffu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
        let e = super::read_vint(&mut &input[..]).unwrap_err();
        assert!(e.message.ends_with("overflows"), "{}", e.message);
        let max = [0xffu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert_eq!(super::read_vint(&mut &max[..]).unwrap(), u64::MAX);
    }

    #[test]
    fn huge_data_size_is_corrupt_not_a_panic() {
        // Header: type 2 (file), flags HFL_DATA, data size u64::MAX.
        let mut body = vec![0x02, 0x02];
        body.extend([0xffu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        let mut archive = super::RAR5_SIGNATURE.to_vec();
        archive.extend([0; 4]);
        archive.push(body.len() as u8);
        archive.extend(body);
        let e = super::ensure_rar_complete(&archive, None).unwrap_err();
        assert!(e.message.ends_with("unexpected end of archive"), "{}", e.message);
    }

    #[tokio::test]
    async fn e2e_0080_016_missing_archive() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/ghost.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
        assert_eq!(e.message, "'/uploads/ghost.rar' not found");
    }

    #[tokio::test]
    async fn e2e_0080_023_part02_rar_is_multi_volume() {
        assert_multi_volume_refused("/uploads/big.part02.rar", "/uploads/big.part02").await;
    }

    #[tokio::test]
    async fn e2e_0080_024_part1x_is_not_a_multi_volume_name() {
        let f = fixture().await;
        for (name, dest) in [
            ("/uploads/report.part1x.rar", "/uploads/report.part1x"),
            ("/uploads/big.part.rar", "/uploads/big.part"),
        ] {
            seed(&f, name, &one_file()).await;
            let v = extract(&f, name).await.unwrap();
            assert_eq!(v["destination"], dest);
            assert_eq!(v["files_written"], 1);
            assert_eq!(read(&f, &format!("{dest}/a.txt")).await, b"hello");
        }
    }

    #[tokio::test]
    async fn e2e_0080_025_upper_case_multi_volume_name() {
        assert_multi_volume_refused("/uploads/BIG.PART3.RAR", "/uploads/BIG.PART3").await;
    }

    fn assert_corrupt(e: &crate::errors::ToolError) {
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(
            e.message.starts_with("archive is corrupt or not a valid rar file:"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn e2e_0080_026_a_text_file_named_rar() {
        let f = fixture().await;
        let e = refuse(&f, "/uploads/fake.rar", b"not an archive".to_vec()).await;
        assert_corrupt(&e);
        assert_absent(&f, &["/uploads/fake"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_027_a_0_byte_rar() {
        let f = fixture().await;
        let e = refuse(&f, "/uploads/empty.rar", Vec::new()).await;
        assert_corrupt(&e);
        assert_absent(&f, &["/uploads/empty"]).await;
    }

    async fn assert_folder_refused(name: &str) {
        let f = fixture().await;
        f.state.stores.client(MOUNT).await.unwrap().makedirs(name, true).await.unwrap();
        let e = extract(&f, name).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert_eq!(e.message, format!("'{name}' is a directory, not an archive file"));
    }

    #[tokio::test]
    async fn e2e_0080_056_path_is_a_folder() {
        assert_folder_refused("/uploads/dir.rar").await;
    }

    #[tokio::test]
    async fn e2e_0080_074_missing_archive_with_an_explicit_des() {
        let f = fixture().await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let e = extract_archive(
            &f.state,
            MOUNT,
            OWNER,
            "/uploads/ghost.rar",
            Some("/out"),
            false,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
        assert_eq!(e.message, "'/uploads/ghost.rar' not found");
        assert_absent(&f, &["/out"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_075_folder_named_with_an_upper_case_suff() {
        assert_folder_refused("/uploads/DIR.RAR").await;
    }

    /// Real archives made by RARLAB `rar` (see tests/fixtures/rar/README.md).
    const V4: &[u8] = include_bytes!("../../tests/fixtures/rar/v4.rar");
    const V5: &[u8] = include_bytes!("../../tests/fixtures/rar/v5.rar");
    const SOLID: &[u8] = include_bytes!("../../tests/fixtures/rar/solid.rar");
    const V4_SECRET: &[u8] = include_bytes!("../../tests/fixtures/rar/v4secret.rar");
    const SECRET: &[u8] = include_bytes!("../../tests/fixtures/rar/secret.rar");
    const HIDDEN: &[u8] = include_bytes!("../../tests/fixtures/rar/hidden.rar");
    const PASSWORD: &str = "s3cret-Ünï";
    const NEEDED: &str =
        "password required to extract this archive; call again with the 'password' parameter";
    const INCORRECT: &str =
        "incorrect password for this archive; call again with the correct 'password' parameter";

    async fn extract_pw(f: &Fixture, path: &str, password: Option<&str>) -> Result<Value> {
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        extract_archive(&f.state, MOUNT, OWNER, path, None, false, password).await
    }

    fn assert_password_refusal(r: Result<Value>, message: &str) {
        let e = r.expect_err("the extraction must be refused");
        assert_eq!(e.code, crate::errors::code::PASSWORD_REQUIRED, "{e:?}");
        assert_eq!(e.message, message);
    }

    /// Capture state is per thread, so the whole body runs on one current thread runtime.
    fn with_capture_lock<F: std::future::Future>(f: F) -> F::Output {
        let _guard = crate::logging::capture::lock_for_test();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// No log event, span or audit line of this session holds `secret`.
    fn assert_never_recorded(f: &Fixture, secret: &str) {
        for ev in crate::logging::capture::events() {
            for v in ev.fields.values() {
                assert!(!v.contains(secret), "event field leaked the password: {v}");
            }
        }
        for sp in crate::logging::capture::spans() {
            for v in sp.fields.values() {
                assert!(!v.contains(secret), "span field leaked the password: {v}");
            }
        }
        for a in f.state.safety.audit(OWNER, MOUNT) {
            assert!(
                !a.path.contains(secret) && !a.detail.contains(secret),
                "audit line leaked the password: {a:?}"
            );
        }
    }

    #[tokio::test]
    async fn e2e_0080_003_right_password_extracts() {
        let f = fixture().await;
        seed(&f, "/uploads/secret.rar", SECRET).await;
        let v = extract_pw(&f, "/uploads/secret.rar", Some(PASSWORD)).await.unwrap();
        assert_eq!(v["files_written"], 1);
        assert_eq!(v["bytes_written"], 10);
        assert_eq!(read(&f, "/uploads/secret/note.txt").await, b"top secret");
    }

    #[tokio::test]
    async fn e2e_0080_004_right_password_on_encrypted_file_nam() {
        let f = fixture().await;
        seed(&f, "/uploads/hidden.rar", HIDDEN).await;
        extract_pw(&f, "/uploads/hidden.rar", Some(PASSWORD)).await.unwrap();
        assert_eq!(read(&f, "/uploads/hidden/note.txt").await, b"top secret");
    }

    #[tokio::test]
    async fn e2e_0080_005_no_password_on_an_encrypted_rar() {
        let f = fixture().await;
        seed(&f, "/uploads/secret.rar", SECRET).await;
        assert_password_refusal(extract_pw(&f, "/uploads/secret.rar", None).await, NEEDED);
        assert_absent(&f, &["/uploads/secret"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_006_no_password_on_encrypted_file_names() {
        let f = fixture().await;
        seed(&f, "/uploads/hidden.rar", HIDDEN).await;
        assert_password_refusal(extract_pw(&f, "/uploads/hidden.rar", None).await, NEEDED);
        assert_absent(&f, &["/uploads/hidden"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_007_wrong_password() {
        let f = fixture().await;
        seed(&f, "/uploads/secret.rar", SECRET).await;
        assert_password_refusal(
            extract_pw(&f, "/uploads/secret.rar", Some("nope")).await,
            INCORRECT,
        );
        assert_absent(&f, &["/uploads/secret"]).await;
    }

    #[test]
    fn e2e_0080_020_success_is_audited_without_the_passw() {
        with_capture_lock(async {
            let f = fixture().await;
            seed(&f, "/uploads/secret.rar", SECRET).await;
            crate::logging::capture::clear();
            extract_pw(&f, "/uploads/secret.rar", Some(PASSWORD)).await.unwrap();
            let log = f.state.safety.audit(OWNER, MOUNT);
            let lines: Vec<_> = log.iter().filter(|e| e.op == "extract_archive").collect();
            assert_eq!(lines.len(), 1, "{log:?}");
            assert_eq!(lines[0].path, "/uploads/secret.rar");
            assert!(lines[0].detail.contains("/uploads/secret"), "{}", lines[0].detail);
            assert!(lines[0].detail.contains('1') && lines[0].detail.contains("10"));
            assert_never_recorded(&f, PASSWORD);
        });
    }

    #[test]
    fn e2e_0080_021_wrong_password_is_not_logged() {
        with_capture_lock(async {
            let f = fixture().await;
            seed(&f, "/uploads/secret.rar", SECRET).await;
            crate::logging::capture::clear();
            let r = extract_pw(&f, "/uploads/secret.rar", Some("nope-12345")).await;
            assert_password_refusal(r, INCORRECT);
            assert_never_recorded(&f, "nope-12345");
            assert!(f.state.safety.audit(OWNER, MOUNT).iter().all(|e| e.op != "extract_archive"));
        });
    }

    #[test]
    fn e2e_0080_030_wrong_password_on_encrypted_file_nam() {
        with_capture_lock(async {
            let f = fixture().await;
            seed(&f, "/uploads/hidden.rar", HIDDEN).await;
            crate::logging::capture::clear();
            let r = extract_pw(&f, "/uploads/hidden.rar", Some("nope-67890")).await;
            assert_password_refusal(r, INCORRECT);
            assert_absent(&f, &["/uploads/hidden"]).await;
            assert_never_recorded(&f, "nope-67890");
        });
    }

    #[tokio::test]
    async fn e2e_0080_062_rar_version_4_extracts() {
        let f = fixture().await;
        seed(&f, "/uploads/v4.rar", V4).await;
        let v = extract(&f, "/uploads/v4.rar").await.unwrap();
        assert_eq!(v["files_written"], 1);
        assert_eq!(v["bytes_written"], 5);
        assert_eq!(read(&f, "/uploads/v4/a.txt").await, b"hello");
    }

    #[tokio::test]
    async fn e2e_0080_063_rar_version_5_extracts() {
        let f = fixture().await;
        seed(&f, "/uploads/v5.rar", V5).await;
        let v = extract(&f, "/uploads/v5.rar").await.unwrap();
        assert_eq!(v["files_written"], 1);
        assert_eq!(v["bytes_written"], 5);
        assert_eq!(read(&f, "/uploads/v5/a.txt").await, b"hello");
    }

    #[tokio::test]
    async fn e2e_0080_064_solid_archive_extracts() {
        let f = fixture().await;
        seed(&f, "/uploads/solid.rar", SOLID).await;
        let v = extract(&f, "/uploads/solid.rar").await.unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(v["bytes_written"], 12);
        assert_eq!(read(&f, "/uploads/solid/a.txt").await, b"hello");
        assert_eq!(read(&f, "/uploads/solid/b.txt").await, b"goodbye");
    }

    const PART: &[u8] = include_bytes!("../../tests/fixtures/rar/part.rar");

    /// The answer of E2E-001, for a plain archive whatever password rode along.
    async fn assert_report_extracted(f: &Fixture, v: &Value) {
        assert_eq!(v["destination"], "/uploads/report");
        assert_eq!(v["files_written"], 2);
        assert_eq!(v["dirs_created"], 2);
        assert_eq!(v["bytes_written"], 12);
        assert_eq!(read(f, "/uploads/report/data/a.txt").await, b"hello");
        assert_eq!(read(f, "/uploads/report/data/b.txt").await, b"goodbye");
    }

    #[tokio::test]
    async fn e2e_0080_031_password_on_a_plain_rar_is_ignored() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        let v = extract_pw(&f, "/uploads/report.rar", Some("whatever")).await.unwrap();
        assert_report_extracted(&f, &v).await;
    }

    #[tokio::test]
    async fn e2e_0080_042_empty_password_on_a_plain_rar_is_ign() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        let v = extract_pw(&f, "/uploads/report.rar", Some("")).await.unwrap();
        assert_report_extracted(&f, &v).await;
    }

    #[tokio::test]
    async fn e2e_0080_050_one_encrypted_entry_among_plain_ones() {
        let f = fixture().await;
        seed(&f, "/uploads/part.rar", PART).await;
        assert_password_refusal(extract_pw(&f, "/uploads/part.rar", None).await, NEEDED);
        assert_absent(&f, &["/uploads/part", "/uploads/part/a.txt"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_051_empty_password_on_an_encrypted_rar() {
        let f = fixture().await;
        seed(&f, "/uploads/secret.rar", SECRET).await;
        assert_password_refusal(extract_pw(&f, "/uploads/secret.rar", Some("")).await, INCORRECT);
        assert_absent(&f, &["/uploads/secret"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_052_partly_encrypted_rar_with_the_right() {
        let f = fixture().await;
        seed(&f, "/uploads/part.rar", PART).await;
        let v = extract_pw(&f, "/uploads/part.rar", Some(PASSWORD)).await.unwrap();
        assert_eq!(v["files_written"], 2);
        assert_eq!(read(&f, "/uploads/part/a.txt").await, b"hello");
        assert_eq!(read(&f, "/uploads/part/b.txt").await, b"bee");
    }

    #[tokio::test]
    async fn e2e_0080_069_empty_password_on_encrypted_file_nam() {
        let f = fixture().await;
        seed(&f, "/uploads/hidden.rar", HIDDEN).await;
        assert_password_refusal(extract_pw(&f, "/uploads/hidden.rar", Some("")).await, INCORRECT);
        assert_absent(&f, &["/uploads/hidden"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_065_wrong_password_on_a_rar_version_4() {
        let f = fixture().await;
        seed(&f, "/uploads/v4secret.rar", V4_SECRET).await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let e = extract_archive(
            &f.state,
            MOUNT,
            OWNER,
            "/uploads/v4secret.rar",
            None,
            false,
            Some("nope"),
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, crate::errors::code::PASSWORD_REQUIRED, "{e:?}");
        assert_eq!(
            e.message,
            "incorrect password for this archive; call again with the correct 'password' parameter"
        );
        assert_absent(&f, &["/uploads/v4secret"]).await;
    }

    // Order of refusals (FR-NEW-015) and membership (FR-NEW-018).

    const BOTH: &[u8] = include_bytes!("../../tests/fixtures/rar/both.rar");
    const HL2: &[u8] = include_bytes!("../../tests/fixtures/rar/hl2.rar");
    const CUT: &[u8] = include_bytes!("../../tests/fixtures/rar/cut.rar");
    const CUTH: &[u8] = include_bytes!("../../tests/fixtures/rar/cuth.rar");
    const CORRUPT_PREFIX: &str = "archive is corrupt or not a valid rar file:";

    /// The caller is checked first, as both doors do, then the engine runs.
    async fn extract_as(f: &Fixture, mount: &str, person: &str, path: &str) -> Result<Value> {
        f.state.authorize(mount, person).await?;
        extract_archive(&f.state, mount, person, path, None, false, None).await
    }

    /// Projects `alpha` (owner `OWNER`, holding `/uploads/<archive>`) and `beta` (owner
    /// carol), with bob and root members of neither.
    async fn two_projects(archive: Option<&str>) -> Fixture {
        let f = Fixture::with_config(|_| {}).await;
        f.seed_project("alpha", OWNER).await;
        f.seed_project("beta", "carol@example.com").await;
        let c = f.state.stores.client("alpha").await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        if let Some(name) = archive {
            c.write_bytes_atomic(name, &report()).await.unwrap();
        }
        f
    }

    fn assert_not_member(e: &crate::errors::ToolError, person: &str) {
        assert_eq!(e.code, crate::errors::code::FORBIDDEN, "{e:?}");
        assert_eq!(e.message, format!("'{person}' is not a member of 'alpha'"));
    }

    async fn assert_alpha_untouched(f: &Fixture) {
        let c = f.state.stores.client("alpha").await.unwrap();
        assert!(!c.exists("/uploads/report").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_0080_047_link_entry_wins_over_a_missing_passw() {
        let f = fixture().await;
        seed(&f, "/uploads/both.rar", BOTH).await;
        let e = extract(&f, "/uploads/both.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_SUPPORTED, "{e:?}");
        assert!(e.message.contains("'l'") && e.message.contains("symlink"), "{}", e.message);
        assert_absent(&f, &["/uploads/both"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_048_corrupt_wins_over_a_wrong_password_w() {
        let f = fixture().await;
        seed(&f, "/uploads/cut.rar", CUT).await;
        let e = extract_pw(&f, "/uploads/cut.rar", Some("nope")).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "{e:?}");
        assert!(e.message.starts_with(CORRUPT_PREFIX), "{}", e.message);
        assert_absent(&f, &["/uploads/cut"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_049_escaping_name_wins_over_a_collision() {
        let f = fixture().await;
        seed(
            &f,
            "/uploads/two.rar",
            &rar5_store(&[RarEntry::file("a.txt", b"a"), RarEntry::file("../x.txt", b"x")]),
        )
        .await;
        seed(&f, "/uploads/two/a.txt", b"old").await;
        let e = extract(&f, "/uploads/two.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PATH_OUT_OF_BOUNDS, "{e:?}");
        assert_eq!(read(&f, "/uploads/two/a.txt").await, b"old");
        assert_absent(&f, &["/uploads/x.txt"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_053_non_member() {
        let f = two_projects(Some("/uploads/report.rar")).await;
        let e = extract_as(&f, "alpha", "bob@example.com", "/uploads/report.rar").await;
        assert_not_member(&e.unwrap_err(), "bob@example.com");
        assert_alpha_untouched(&f).await;
    }

    /// Platform administration is not file access: root is no member of `alpha`.
    #[tokio::test]
    async fn e2e_0080_054_platform_administrator_without_membe() {
        let f = two_projects(Some("/uploads/report.rar")).await;
        let e = extract_as(&f, "alpha", "root@example.com", "/uploads/report.rar").await;
        assert_not_member(&e.unwrap_err(), "root@example.com");
        assert_alpha_untouched(&f).await;
    }

    #[tokio::test]
    async fn e2e_0080_055_member_of_another_project() {
        let f = two_projects(Some("/uploads/report.rar")).await;
        let e = extract_as(&f, "alpha", "carol@example.com", "/uploads/report.rar").await;
        assert_not_member(&e.unwrap_err(), "carol@example.com");
        assert_alpha_untouched(&f).await;
    }

    /// The caller is refused before the path is looked at, so a missing archive does
    /// not tell a stranger what the project holds.
    #[tokio::test]
    async fn e2e_0080_057_non_member_asking_for_a_missing_arch() {
        let f = two_projects(None).await;
        let e = extract_as(&f, "alpha", "bob@example.com", "/uploads/ghost.rar").await;
        assert_not_member(&e.unwrap_err(), "bob@example.com");
    }

    #[tokio::test]
    async fn e2e_0080_058_truncated_archive_with_encrypted_fil() {
        let f = fixture().await;
        seed(&f, "/uploads/cuth.rar", CUTH).await;
        assert_password_refusal(extract_pw(&f, "/uploads/cuth.rar", Some("nope")).await, INCORRECT);
        assert_absent(&f, &["/uploads/cuth"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_059_encrypted_file_names_right_password() {
        let f = fixture().await;
        seed(&f, "/uploads/hl2.rar", HL2).await;
        let e = extract_pw(&f, "/uploads/hl2.rar", Some(PASSWORD)).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::NOT_SUPPORTED, "{e:?}");
        assert!(e.message.contains("'l'") && e.message.contains("symlink"), "{}", e.message);
        assert_absent(&f, &["/uploads/hl2"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_060_truncated_archive_with_readable_name() {
        let f = fixture().await;
        seed(&f, "/uploads/cut2.rar", CUT).await;
        let e = extract(&f, "/uploads/cut2.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "{e:?}");
        assert!(e.message.starts_with(CORRUPT_PREFIX), "{}", e.message);
        assert_absent(&f, &["/uploads/cut2"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_061_destination_outside_the_project() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        let e = extract_archive(
            &f.state,
            MOUNT,
            OWNER,
            "/uploads/report.rar",
            Some("../../x"),
            false,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, crate::errors::code::PATH_OUT_OF_BOUNDS, "{e:?}");
        assert_eq!(e.message, "path escapes the volume root: ../../x");
        assert_absent(&f, &["/uploads/report", "/x"]).await;
    }

    /// Encrypted data with readable names, the first entry declaring 10 bytes and
    /// holding 1 000 000: the password is asked for before any entry is decoded.
    #[tokio::test]
    async fn e2e_0080_066_missing_password_wins_over_an_oversi() {
        let f = fixture().await;
        seed(
            &f,
            "/uploads/encbomb.rar",
            &rar5_store(&[RarEntry::encrypted_declaring("big.bin", 10, &vec![7u8; 1_000_000])]),
        )
        .await;
        assert_password_refusal(extract_pw(&f, "/uploads/encbomb.rar", None).await, NEEDED);
        assert_absent(&f, &["/uploads/encbomb"]).await;
    }

    #[tokio::test]
    async fn e2e_0080_067_oversized_entry_wins_over_a_collisio() {
        let f = fixture().await;
        seed(
            &f,
            "/uploads/bomb.rar",
            &rar5_store(&[RarEntry::declaring("big.bin", 10, &vec![7u8; 1_000_000])]),
        )
        .await;
        seed(&f, "/uploads/bomb/big.bin", b"old").await;
        let e = extract(&f, "/uploads/bomb.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "{e:?}");
        assert!(e.message.contains("big.bin"), "{}", e.message);
        assert_eq!(read(&f, "/uploads/bomb/big.bin").await, b"old");
    }

    #[tokio::test]
    async fn e2e_0085_mod_001_the_storage_limit_wins_over_a_collis() {
        let f = fixture().await;
        seed(&f, "/uploads/report.rar", &report()).await;
        seed(&f, "/uploads/report/data/a.txt", b"old").await;
        limit_remaining(&f, 5).await;
        let e = extract(&f, "/uploads/report.rar").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
        assert!(e.message.ends_with("9 bytes requested"), "{}", e.message);
        assert_eq!(read(&f, "/uploads/report/data/a.txt").await, b"old");
        assert_eq!(remaining_storage(&f).await, 5);
    }

    #[tokio::test]
    async fn e2e_0080_081_the_project_refusal_comes_first() {
        let f = fixture_with_allowance(5).await;
        seed(&f, "/uploads/report.rar", &report()).await;
        limit_remaining(&f, 5).await;
        let v = extract(&f, "/uploads/report.rar").await;
        project_refusal(&v);
        assert_eq!(remaining_storage(&f).await, 5);
        assert_eq!(allowance(&f), 5);
        assert_absent(&f, &["/uploads/report"]).await;
    }

    #[test]
    fn multi_volume_name_edge_cases() {
        use crate::tools::archive::is_multi_volume_name as m;
        assert!(m("a.part1.rar"));
        assert!(m("a.PART10.RAR"));
        assert!(m("big.part01.rar"));
        assert!(m(".part1.rar"));
        assert!(!m("a.part.rar"));
        assert!(!m("a.part1x.rar"));
        assert!(!m("a.partx1.rar"));
        // No dot before `part`: the spec says the name ends in `.part` digits `.rar`.
        assert!(!m("part1.rar"));
        assert!(!m("a.part1.rar.zip"));
        assert!(!m("a.part1"));
    }

    /// Pins `rar5_kdf` on an archive RAR 7.23 made: the derived check value equals the
    /// one stored in `hidden.rar`, and the key decrypts its first header to a matching CRC.
    #[test]
    fn rar5_check_value_of_hidden_rar_matches() {
        use crate::tools::archive_rar_crypt::{aes256_cbc_decrypt, rar5_kdf};
        let hidden: &[u8] = include_bytes!("../../tests/fixtures/rar/hidden.rar");
        // Encryption header at 8: CRC32, size 33, type 4, flags 0, version 0, check
        // value present, lg2_count, salt, check value, its sum.
        assert_eq!(&hidden[12..17], &[0x21, 0x04, 0x00, 0x00, 0x01]);
        let lg2_count = hidden[17];
        let salt: [u8; 16] = hidden[18..34].try_into().unwrap();
        let (key, check) = rar5_kdf("s3cret-Ünï", &salt, lg2_count);
        assert_eq!(check, hidden[34..42]);
        assert_ne!(rar5_kdf("s3cret-Üni", &salt, lg2_count).1, check);

        let iv: [u8; 16] = hidden[46..62].try_into().unwrap();
        let mut first = hidden[62..78].to_vec();
        aes256_cbc_decrypt(&key, &iv, &mut first);
        let size = usize::from(first[4]);
        assert!(size < 0x80, "a one byte vint");
        let padded = (5 + size).div_ceil(16) * 16;
        let mut plain = hidden[62..62 + padded].to_vec();
        aes256_cbc_decrypt(&key, &iv, &mut plain);
        let mut crc = flate2::Crc::new();
        crc.update(&plain[4..5 + size]);
        assert_eq!(crc.sum().to_le_bytes(), plain[..4]);
    }

    /// Decrypts the first encrypted header of a RAR4 archive made with `-hp`: marker and
    /// main header (20 bytes), then the 8 byte salt and the ciphertext of a 50 byte file
    /// header (4 AES blocks). Checks the CRC16 and the name `a.txt`.
    fn assert_rar3_key_decrypts_first_header(archive: &[u8], password: &str) {
        use crate::tools::archive_rar_crypt::{aes128_cbc_decrypt, rar3_kdf};
        assert_eq!(&archive[7..14], &[0xce, 0x99, 0x73, 0x80, 0x00, 0x0d, 0x00]);
        let salt: [u8; 8] = archive[20..28].try_into().unwrap();
        let (key, iv) = rar3_kdf(password, &salt);
        let mut plain = archive[28..92].to_vec();
        aes128_cbc_decrypt(&key, &iv, &mut plain);
        assert_eq!(plain[2], 0x74, "a file header");
        let size = usize::from(u16::from_le_bytes([plain[5], plain[6]]));
        assert_eq!(size, 50);
        let mut crc = flate2::Crc::new();
        crc.update(&plain[2..size]);
        assert_eq!(crc.sum().to_le_bytes()[..2], plain[..2]);
        assert_eq!(&plain[32..37], b"a.txt");
    }

    /// Pins `rar3_kdf` on `hp4.rar` (RAR 6.24): its password is too short for UnRAR's
    /// schedule write back, so this is the plain RAR 3 derivation.
    #[test]
    fn rar3_key_decrypts_the_first_header_of_hp4() {
        let hp4: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4.rar");
        assert_eq!(&hp4[20..28], &[0xf3, 0x68, 0x27, 0x2b, 0x78, 0xec, 0xe5, 0x69]);
        assert_rar3_key_decrypts_first_header(hp4, "s3cret-Ünï");
    }

    /// A RAR4 header that fails its CRC once decrypted is UnRAR's to answer, and so is an
    /// archive read with no password or an empty one: the 108 byte cut of `hp4.rar`,
    /// refused with the right password, passes the walk in each case. Flipping a byte of
    /// the second AES block of the first header garbles it without touching its size.
    #[test]
    fn rar4_encrypted_headers_left_to_the_library() {
        let hp4: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4.rar");
        let cut = &hp4[..108];
        let e = super::ensure_rar_complete(cut, Some("s3cret-Ünï")).unwrap_err();
        assert_eq!(
            e.message,
            "archive is corrupt or not a valid rar file: unexpected end of archive"
        );
        let mut garbled = cut.to_vec();
        garbled[50] ^= 1;
        super::ensure_rar_complete(&garbled, Some("s3cret-Ünï")).unwrap();
        super::ensure_rar_complete(cut, None).unwrap();
        super::ensure_rar_complete(cut, Some("")).unwrap();
    }

    /// Pins the write back of `sha1_process_rar29` on `hp4long.rar` (RAR 6.24): 80 bytes
    /// of UTF-16 password plus 8 of salt span a whole block read straight from the
    /// password buffer, so a plain SHA-1 derivation gives another key here.
    #[test]
    fn rar3_key_with_a_long_password_decrypts_hp4long() {
        let hp4long: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4long.rar");
        assert_rar3_key_decrypts_first_header(hp4long, "CorrectHorseBatteryStapleÜnï0123456789ab");
    }
}
