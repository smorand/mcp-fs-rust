//! Test only RAR5 archive builder (SPEC-0080 DEC-009). `rar` is not installed and
//! no Rust crate writes RAR, so plain cases are assembled byte by byte in the
//! STORE method (no compression). The real `unrar` reader reading this output back
//! is what checks the builder (E2E-001).

/// RAR5 marker block: every RAR5 archive starts with these eight bytes.
const RAR5_SIGNATURE: &[u8] = b"Rar!\x1A\x07\x01\x00";
const HEAD_MAIN: u64 = 1;
const HEAD_FILE: u64 = 2;
const HEAD_END: u64 = 5;
/// Header flags: an extra area follows the type specific fields.
const HFL_EXTRA: u64 = 0x0001;
/// Header flags: a data area follows the header.
const HFL_DATA: u64 = 0x0002;
/// File flags: the entry is a directory.
const FHFL_DIRECTORY: u64 = 0x0001;
/// File flags: the data CRC32 field is present.
const FHFL_CRC32: u64 = 0x0004;
/// Host OS Unix, so `file_attr` is read as a POSIX mode.
const HOST_UNIX: u64 = 1;
const MODE_FILE: u64 = 0o100_644;
const MODE_DIR: u64 = 0o040_755;
const MODE_SYMLINK: u64 = 0o120_777;
/// File extra record type: file system redirection (link).
const EXTRA_REDIRECTION: u64 = 5;
const REDIR_UNIX_SYMLINK: u64 = 1;
const REDIR_HARD_LINK: u64 = 4;

/// What one entry of the built archive is.
// Symlink and HardLink are built here but first used by the entry rule stories
// (US-0171 and later), so this story leaves them unconstructed.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum RarKind {
    File,
    Dir,
    Symlink(String),
    HardLink(String),
}

/// One entry of the built archive, in archive order.
#[derive(Debug, Clone)]
pub(crate) struct RarEntry {
    pub(crate) name: String,
    pub(crate) kind: RarKind,
    pub(crate) data: Vec<u8>,
}

impl RarEntry {
    pub(crate) fn file(name: &str, data: &[u8]) -> Self {
        Self { name: name.to_string(), kind: RarKind::File, data: data.to_vec() }
    }

    pub(crate) fn dir(name: &str) -> Self {
        Self { name: name.to_string(), kind: RarKind::Dir, data: Vec::new() }
    }
}

/// A valid RAR5 archive holding `entries` stored without compression.
pub(crate) fn rar5_store(entries: &[RarEntry]) -> Vec<u8> {
    let mut out = RAR5_SIGNATURE.to_vec();
    // Main archive header: archive flags 0 (single volume, not solid).
    push_header(&mut out, HEAD_MAIN, 0, &[], &vint(0), &[]);
    for entry in entries {
        push_file(&mut out, entry);
    }
    // End of archive header: flags 0, this is the last volume.
    push_header(&mut out, HEAD_END, 0, &[], &vint(0), &[]);
    out
}

fn push_file(out: &mut Vec<u8>, entry: &RarEntry) {
    let (file_flags, mode, data, extra) = match &entry.kind {
        RarKind::File => (FHFL_CRC32, MODE_FILE, entry.data.as_slice(), Vec::new()),
        RarKind::Dir => (FHFL_DIRECTORY, MODE_DIR, &[][..], Vec::new()),
        RarKind::Symlink(target) => {
            (FHFL_CRC32, MODE_SYMLINK, &[][..], redirection(REDIR_UNIX_SYMLINK, target))
        }
        RarKind::HardLink(target) => {
            (FHFL_CRC32, MODE_FILE, &[][..], redirection(REDIR_HARD_LINK, target))
        }
    };
    let mut fields = Vec::new();
    fields.extend(vint(file_flags));
    fields.extend(vint(data.len() as u64));
    fields.extend(vint(mode));
    if file_flags & FHFL_CRC32 != 0 {
        fields.extend(crc32(data).to_le_bytes());
    }
    // Compression info 0: algorithm version 0 (RAR 5.0), not solid, method STORE.
    fields.extend(vint(0));
    fields.extend(vint(HOST_UNIX));
    fields.extend(vint(entry.name.len() as u64));
    fields.extend(entry.name.as_bytes());
    let flags = HFL_DATA | if extra.is_empty() { 0 } else { HFL_EXTRA };
    let mut pre = Vec::new();
    if !extra.is_empty() {
        pre.extend(vint(extra.len() as u64));
    }
    pre.extend(vint(data.len() as u64));
    push_header(out, HEAD_FILE, flags, &pre, &fields, &extra);
    out.extend(data);
}

/// A redirection extra record: size, type, redirection type, flags, target.
fn redirection(kind: u64, target: &str) -> Vec<u8> {
    let mut body = vint(EXTRA_REDIRECTION);
    body.extend(vint(kind));
    body.extend(vint(0));
    body.extend(vint(target.len() as u64));
    body.extend(target.as_bytes());
    let mut record = vint(body.len() as u64);
    record.extend(body);
    record
}

/// `CRC32 | size | type | flags | pre | fields | extra`, where size counts from type
/// to the end and the CRC covers size onwards.
fn push_header(out: &mut Vec<u8>, kind: u64, flags: u64, pre: &[u8], fields: &[u8], extra: &[u8]) {
    let mut body = vint(kind);
    body.extend(vint(flags));
    body.extend(pre);
    body.extend(fields);
    body.extend(extra);
    let mut sized = vint(body.len() as u64);
    sized.extend(body);
    out.extend(crc32(&sized).to_le_bytes());
    out.extend(sized);
}

/// RAR5 variable length integer: 7 bits per byte, low first, high bit continues.
fn vint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// Bitwise CRC32 (IEEE, reflected): no crc crate is a dependency and test archives
/// are tiny.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_reference_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn vint_encodes_seven_bits_per_byte() {
        assert_eq!(vint(0), vec![0]);
        assert_eq!(vint(127), vec![0x7f]);
        assert_eq!(vint(128), vec![0x80, 0x01]);
    }
}
