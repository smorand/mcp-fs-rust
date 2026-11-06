//! `.rar` decoding for `fs.extract_archive` (SPEC-0080), compiled only with the opt in
//! `rar` feature: the reader is the `unrar` crate, a wrapper of RARLAB's UnRAR whose
//! source licence is non free (DEC-001).

use crate::errors::{Result, ToolError};
use crate::tools::archive::{DecodedEntry, corrupt, ensure_entry_path_safe};

/// Name the archive gets inside its per call temp dir; the extension is what the
/// UnRAR library expects, the name itself never reaches the caller.
const TEMP_NAME: &str = "archive.rar";

/// Decode every entry of a RAR archive held in `bytes`, in archive order. Blocking:
/// the caller runs it inside `spawn_blocking`. Time is linear in the archive plus the
/// unpacked size; memory is the archive on disk once plus every decoded entry in memory,
/// as for zip and 7z, because the admission runs after the decode (BL-0125).
pub(crate) fn decode_rar(bytes: &[u8], password: Option<&str>) -> Result<Vec<DecodedEntry>> {
    // The crate only opens a path (DEC-003), so the bytes go to a private temp dir
    // that is removed when `dir` drops, on every return path.
    let dir = tempfile::tempdir()
        .map_err(|e| ToolError::internal(format!("rar temp dir failed: {e}")))?;
    let path = dir.path().join(TEMP_NAME);
    std::fs::write(&path, bytes)
        .map_err(|e| ToolError::internal(format!("rar temp file failed: {e}")))?;

    // Entry rules run over a listing pass first, so a link or an escaping name
    // anywhere in the archive is refused before one byte is decoded (FR-NEW-015).
    for header in open(&path, password).open_for_listing().map_err(|e| corrupt("rar", e))? {
        let header = header.map_err(|e| corrupt("rar", e))?;
        ensure_entry_allowed(&header)?;
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
        // An encrypted entry fails for other reasons too (password handling arrives with
        // US-0175), so only a plain entry's read error is the oversize signal.
        let encrypted = entry.is_encrypted();
        let (data, next) = header.read().map_err(|e| {
            if encrypted { corrupt("rar", e) } else { oversized(&rel_path, declared_size) }
        })?;
        if data.len() as u64 != declared_size {
            return Err(oversized(&rel_path, declared_size));
        }
        entries.push(DecodedEntry { rel_path, is_dir: false, declared_size, bytes: data });
        cursor = next;
    }
    Ok(entries)
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
}
