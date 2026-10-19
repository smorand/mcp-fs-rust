//! Whole archive header consistency check (SPEC-0041).
//!
//! A zip names every entry twice: once in its own local header, once in the
//! central directory. Readers that trust different copies see different
//! archives, so an archive whose two copies disagree (or that holds two
//! entries of one name, or entries sharing bytes) is refused before anything
//! is written or converted. The check lives here once; each caller formats
//! its own message prefix around the returned reason (DEC-002).

/// The seven zip based document extensions (FR-NEW-002, FR-NEW-005).
pub(crate) const OOXML_EXTS: &[&str] =
    &[".docx", ".xlsx", ".xlsm", ".pptx", ".pptm", ".potx", ".ppsx"];

/// zip's `Display` prefix for `InvalidArchive`; stripped so every caller
/// shows the reason itself, e.g. `header inconsistency detected: ...`.
const INVALID_PREFIX: &str = "invalid Zip archive: ";

/// `Some(reason)` when zip's whole archive check fails. Reads headers only,
/// never entry content, so it needs no password (FR-NEW-003).
pub(crate) fn header_inconsistency<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Option<String> {
    let err = archive.validate_header_consistency().err()?;
    let text = err.to_string();
    Some(text.strip_prefix(INVALID_PREFIX).map(str::to_owned).unwrap_or(text))
}

/// `Some(reason)` when `path` has one of [`OOXML_EXTS`], the bytes open as a
/// zip, and the check fails. A zip that does not open returns `None`: that
/// case keeps its existing handling at the caller.
pub(crate) fn ooxml_header_inconsistency(path: &str, data: &[u8]) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if !OOXML_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        return None;
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).ok()?;
    header_inconsistency(&mut archive)
}

/// Byte patchers turning a well formed `zip::ZipWriter` output into the
/// malformed archives the SPEC-0041 tests need (DEC-007). Only plain 32 bit
/// archives are handled: zip64 records never appear in the test fixtures.
#[cfg(test)]
pub(crate) mod craft {
    const LOCAL_SIG: [u8; 4] = 0x0403_4b50u32.to_le_bytes();
    const CENTRAL_SIG: [u8; 4] = 0x0201_4b50u32.to_le_bytes();
    const END_SIG: [u8; 4] = 0x0605_4b50u32.to_le_bytes();

    fn u16_at(b: &[u8], at: usize) -> usize {
        usize::from(u16::from_le_bytes([b[at], b[at + 1]]))
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn put_u16(b: &mut [u8], at: usize, v: usize) {
        b[at..at + 2].copy_from_slice(&u16::try_from(v).unwrap().to_le_bytes());
    }

    fn put_u32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Offset of the end of central directory record, searched backwards so
    /// a signature inside entry data never matches.
    fn end_record(b: &[u8]) -> usize {
        (0..=b.len() - 22).rev().find(|&i| b[i..i + 4] == END_SIG).expect("end record")
    }

    /// Offsets of every central directory record, walked from the end record
    /// rather than scanned, so entry data cannot fake one.
    fn central_records(b: &[u8]) -> Vec<usize> {
        let end = end_record(b);
        let count = u16_at(b, end + 10);
        let mut at = u32_at(b, end + 16) as usize;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            assert_eq!(b[at..at + 4], CENTRAL_SIG, "central record");
            out.push(at);
            at += 46 + u16_at(b, at + 28) + u16_at(b, at + 30) + u16_at(b, at + 32);
        }
        out
    }

    /// Offset of the local header the central record named `name` points at.
    fn local_of(b: &[u8], name: &str) -> usize {
        let cd = central_records(b)
            .into_iter()
            .find(|&c| &b[c + 46..c + 46 + u16_at(b, c + 28)] == name.as_bytes())
            .unwrap_or_else(|| panic!("no central record named {name}"));
        let local = u32_at(b, cd + 42) as usize;
        assert_eq!(b[local..local + 4], LOCAL_SIG, "local header");
        local
    }

    /// Renames the local header of the entry the directory calls `from` to
    /// `to`, of any length: the name is spliced, its length field fixed, and
    /// every later local header offset plus the directory offset shifted.
    pub(crate) fn rename_local(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
        let local = local_of(bytes, from);
        let old_len = u16_at(bytes, local + 26);
        let name_at = local + 30;
        let mut out = Vec::with_capacity(bytes.len() + to.len());
        out.extend_from_slice(&bytes[..name_at]);
        out.extend_from_slice(to.as_bytes());
        out.extend_from_slice(&bytes[name_at + old_len..]);
        put_u16(&mut out, local + 26, to.len());
        let delta = i64::try_from(to.len()).unwrap() - i64::try_from(old_len).unwrap();
        let shift = |v: u32| u32::try_from(i64::from(v) + delta).unwrap();
        // The end record first: walking the directory needs its true offset.
        let end = end_record(&out);
        let cd_start = u32_at(&out, end + 16);
        put_u32(&mut out, end + 16, shift(cd_start));
        for cd in central_records(&out) {
            let off = u32_at(&out, cd + 42);
            if off as usize > local {
                put_u32(&mut out, cd + 42, shift(off));
            }
        }
        out
    }

    /// Sets the DOS modification time of the local header of `name` only.
    pub(crate) fn set_local_mtime(bytes: &[u8], name: &str, dos_time: u16) -> Vec<u8> {
        let mut out = bytes.to_vec();
        let local = local_of(&out, name);
        put_u16(&mut out, local + 10, usize::from(dos_time));
        out
    }

    /// Sets the compressed size of the local header of `name` only.
    pub(crate) fn set_local_compressed_size(bytes: &[u8], name: &str, size: u32) -> Vec<u8> {
        let mut out = bytes.to_vec();
        let local = local_of(&out, name);
        put_u32(&mut out, local + 18, size);
        out
    }

    /// Adds a directory row for a local entry nested inside the first
    /// entry's data (the first entry must be stored and its content a whole
    /// local record, e.g. another stored archive's leading bytes). Each row
    /// then matches its own local header exactly, so only the shared bytes
    /// are wrong: this reaches zip's overlap check, not its name check.
    pub(crate) fn overlap(bytes: &[u8]) -> Vec<u8> {
        let first = central_records(bytes)[0];
        let outer = u32_at(bytes, first + 42) as usize;
        let inner = outer + 30 + u16_at(bytes, outer + 26) + u16_at(bytes, outer + 28);
        assert_eq!(bytes[inner..inner + 4], LOCAL_SIG, "nested local header");
        let (n, e) = (u16_at(bytes, inner + 26), u16_at(bytes, inner + 28));
        let mut row = Vec::with_capacity(46 + n + e);
        row.extend_from_slice(&CENTRAL_SIG);
        row.extend_from_slice(&bytes[inner + 4..inner + 6]);
        row.extend_from_slice(&bytes[inner + 4..inner + 30]);
        row.extend_from_slice(&[0u8; 10]);
        row.extend_from_slice(&u32::try_from(inner).unwrap().to_le_bytes());
        row.extend_from_slice(&bytes[inner + 30..inner + 30 + n + e]);

        let end = end_record(bytes);
        let mut out = Vec::with_capacity(bytes.len() + row.len());
        out.extend_from_slice(&bytes[..end]);
        out.extend_from_slice(&row);
        out.extend_from_slice(&bytes[end..]);
        let end = end + row.len();
        let count = u16_at(&out, end + 10) + 1;
        put_u16(&mut out, end + 8, count);
        put_u16(&mut out, end + 10, count);
        let size = u32_at(&out, end + 12) + u32::try_from(row.len()).unwrap();
        put_u32(&mut out, end + 12, size);
        out
    }

    /// Local record bytes (header plus data) of the first entry of a stored
    /// archive, ready to be nested as another entry's content by [`overlap`].
    pub(crate) fn first_local_record(bytes: &[u8]) -> Vec<u8> {
        let first = central_records(bytes)[0];
        let local = u32_at(bytes, first + 42) as usize;
        let data = local + 30 + u16_at(bytes, local + 26) + u16_at(bytes, local + 28);
        bytes[local..data + u32_at(bytes, first + 20) as usize].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read as _, Write as _};

    fn build(entries: &[(&str, &[u8])], method: zip::CompressionMethod) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, content) in entries {
                let opts = zip::write::SimpleFileOptions::default().compression_method(method);
                w.start_file(*name, opts).unwrap();
                w.write_all(content).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    fn open(bytes: &[u8]) -> zip::ZipArchive<Cursor<&[u8]>> {
        zip::ZipArchive::new(Cursor::new(bytes)).unwrap()
    }

    fn read_entry(bytes: &[u8], name: &str) -> Vec<u8> {
        let mut a = open(bytes);
        let mut out = Vec::new();
        a.by_name(name).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn well_formed_archive_has_no_inconsistency() {
        let bytes =
            build(&[("a.txt", b"alpha"), ("b.txt", b"beta")], zip::CompressionMethod::Deflated);
        assert_eq!(header_inconsistency(&mut open(&bytes)), None);
    }

    #[test]
    fn rename_local_longer_name_keeps_later_entries_readable_and_is_flagged() {
        let bytes =
            build(&[("a.txt", b"alpha"), ("b.txt", b"beta")], zip::CompressionMethod::Deflated);
        let crafted = craft::rename_local(&bytes, "a.txt", "../../evil.txt");
        assert_eq!(crafted.len(), bytes.len() + 9);
        // The second entry and the directory moved with the splice.
        assert_eq!(read_entry(&crafted, "b.txt"), b"beta");
        let reason = header_inconsistency(&mut open(&crafted)).unwrap();
        assert!(reason.starts_with("header inconsistency detected: "), "{reason}");
        assert!(reason.contains("filename mismatch"), "{reason}");
    }

    #[test]
    fn rename_local_shorter_name_is_flagged() {
        let bytes =
            build(&[("long_name.txt", b"x"), ("b.txt", b"y")], zip::CompressionMethod::Stored);
        let crafted = craft::rename_local(&bytes, "long_name.txt", "s.txt");
        assert_eq!(read_entry(&crafted, "b.txt"), b"y");
        assert!(header_inconsistency(&mut open(&crafted)).unwrap().contains("filename mismatch"));
    }

    #[test]
    fn set_local_mtime_is_flagged_as_time_mismatch() {
        let bytes = build(&[("a.txt", b"alpha")], zip::CompressionMethod::Stored);
        let crafted = craft::set_local_mtime(&bytes, "a.txt", 0x1234);
        let reason = header_inconsistency(&mut open(&crafted)).unwrap();
        assert!(reason.contains("last modified time mismatch"), "{reason}");
    }

    #[test]
    fn set_local_compressed_size_is_flagged_as_size_mismatch() {
        let bytes = build(&[("a.txt", b"alpha")], zip::CompressionMethod::Stored);
        let crafted = craft::set_local_compressed_size(&bytes, "a.txt", 4);
        let reason = header_inconsistency(&mut open(&crafted)).unwrap();
        assert!(reason.contains("compressed size mismatch"), "{reason}");
    }

    #[test]
    fn overlap_reaches_the_overlap_check() {
        let inner = craft::first_local_record(&build(
            &[("b.txt", b"beta")],
            zip::CompressionMethod::Stored,
        ));
        let outer = build(&[("a.txt", &inner)], zip::CompressionMethod::Stored);
        let crafted = craft::overlap(&outer);
        let mut a = open(&crafted);
        assert_eq!(a.len(), 2);
        assert_eq!(read_entry(&crafted, "b.txt"), b"beta");
        assert_eq!(
            header_inconsistency(&mut a).as_deref(),
            Some("header inconsistency detected: archive contains overlapping entries")
        );
    }

    #[test]
    fn ooxml_check_ignores_other_extensions() {
        let crafted = craft::rename_local(
            &build(&[("word/document.xml", b"x")], zip::CompressionMethod::Stored),
            "word/document.xml",
            "word/other.xml",
        );
        assert_eq!(ooxml_header_inconsistency("/d/a.zip", &crafted), None);
        assert_eq!(ooxml_header_inconsistency("/d/a.txt", &crafted), None);
    }

    #[test]
    fn ooxml_check_ignores_bytes_that_are_not_a_zip() {
        assert_eq!(ooxml_header_inconsistency("/d/a.docx", b"not a zip at all"), None);
    }

    #[test]
    fn ooxml_check_flags_a_crafted_docx_in_any_case() {
        let bytes = build(&[("word/document.xml", b"x")], zip::CompressionMethod::Stored);
        assert_eq!(ooxml_header_inconsistency("/d/a.docx", &bytes), None);
        let crafted = craft::rename_local(&bytes, "word/document.xml", "word/other.xml");
        let reason = ooxml_header_inconsistency("/d/A.DOCX", &crafted).unwrap();
        assert!(reason.starts_with("header inconsistency detected: "), "{reason}");
    }
}
