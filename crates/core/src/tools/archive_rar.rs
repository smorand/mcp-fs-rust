//! `.rar` decoding for `fs.extract_archive` (SPEC-0080), compiled only with the opt in
//! `rar` feature: the reader is the `unrar` crate, a wrapper of RARLAB's UnRAR whose
//! source licence is non free (DEC-001).

use crate::errors::{Result, ToolError};
use crate::tools::archive::{
    DecodedEntry, corrupt, ensure_entry_path_safe, password_required_message,
};

/// Name the archive gets inside its per call temp dir; the extension is what the
/// UnRAR library expects, the name itself never reaches the caller.
const TEMP_NAME: &str = "archive.rar";

/// Decode every entry of a RAR archive held in `bytes`, in archive order. Blocking:
/// the caller runs it inside `spawn_blocking`. Time is linear in the archive plus the
/// unpacked size; memory is the archive on disk once plus every decoded entry in memory,
/// as for zip and 7z, because the admission runs after the decode (BL-0125).
pub(crate) fn decode_rar(bytes: &[u8], password: Option<&str>) -> Result<Vec<DecodedEntry>> {
    // UnRAR reads a truncated archive as a shorter one or as a CRC error, neither of
    // which says corrupt (FR-NEW-011), so the block layout is checked first.
    ensure_rar5_complete(bytes)?;
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
    for header in open(&path, password).open_for_listing().map_err(list_err)? {
        let header = header.map_err(list_err)?;
        ensure_entry_allowed(&header)?;
        any_encrypted |= header.is_encrypted();
    }
    // Decided before one byte is read, and after the entry rules (FR-NEW-015 item 7
    // after 6), so a plain entry is never written next to an encrypted one (FR-NEW-002).
    if any_encrypted && password.is_none() {
        return Err(password_required_message(None));
    }

    let mut cursor = open(&path, password).open_for_processing().map_err(|e| corrupt("rar", e))?;
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
        entries.push(DecodedEntry { rel_path, is_dir: false, declared_size, bytes: data });
        cursor = next;
    }
    Ok(entries)
}

/// RAR5 marker block; RAR 1.5 to 4.x archives start with `Rar!\x1a\x07\x00`.
const RAR5_SIGNATURE: &[u8] = b"Rar!\x1a\x07\x01\x00";
/// Header type of the archive encryption block: every header after it is encrypted,
/// so the walk cannot read further.
const HEAD_CRYPT: u64 = 4;
/// Header type of the end of archive block, the last block of a complete volume.
const HEAD_END: u64 = 5;
const HFL_EXTRA: u64 = 0x1;
const HFL_DATA: u64 = 0x2;
/// A vint carries 7 bits per byte, so 10 bytes hold any u64.
const VINT_MAX_BYTES: usize = 10;
const TRUNCATED: &str = "unexpected end of archive";

/// Walk the RAR5 block chain and refuse an archive whose last block runs past the end
/// of the input, or that ends before its end of archive block. Linear in the number of
/// blocks, no copies. A non RAR5 input is left to UnRAR: RAR4 truncation is not
/// detected (open limitation), and neither is a cut after an archive encryption
/// header, whose following headers cannot be read without the password.
fn ensure_rar5_complete(bytes: &[u8]) -> Result<()> {
    let Some(mut rest) = bytes.strip_prefix(RAR5_SIGNATURE) else {
        return Ok(());
    };
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
        if kind == HEAD_END || kind == HEAD_CRYPT {
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
    ToolError::invalid_argument(format!(
        "archive entry '{rel_path}' decompressed to more bytes than declared {declared}"
    ))
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
        let e = super::decode_rar(b"not a rar archive", None).unwrap_err();
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
        super::ensure_rar5_complete(&full).unwrap();
        for len in super::RAR5_SIGNATURE.len()..full.len() {
            let e = super::ensure_rar5_complete(&full[..len]).unwrap_err();
            assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "len {len}");
            assert!(
                e.message.starts_with("archive is corrupt or not a valid rar file:"),
                "len {len}"
            );
        }
    }

    #[test]
    fn non_rar5_input_is_left_to_the_library() {
        super::ensure_rar5_complete(b"Rar!\x1a\x07\x00").unwrap();
        super::ensure_rar5_complete(b"").unwrap();
        super::ensure_rar5_complete(b"Rar!\x1a\x07\x01").unwrap();
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
        assert!(super::ensure_rar5_complete(&archive).is_err());
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
        let e = super::ensure_rar5_complete(&archive).unwrap_err();
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
}
