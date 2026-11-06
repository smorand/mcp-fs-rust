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

    let archive = match password {
        Some(pw) => unrar::Archive::with_password(&path, pw),
        None => unrar::Archive::new(&path),
    };
    let mut cursor = archive.open_for_processing().map_err(|e| corrupt("rar", e))?;
    let mut entries = Vec::new();
    // Entries are read in archive order, because a solid archive can only be
    // decoded front to back.
    while let Some(header) = cursor.read_header().map_err(|e| corrupt("rar", e))? {
        let entry = header.entry();
        // UnRAR hands back the platform separator; the volume always speaks `/`.
        let rel_path = entry.filename.to_string_lossy().replace('\\', "/");
        ensure_entry_path_safe(&rel_path)?;
        let declared_size = entry.unpacked_size;
        if entry.is_directory() {
            entries.push(DecodedEntry { rel_path, is_dir: true, declared_size, bytes: Vec::new() });
            cursor = header.skip().map_err(|e| corrupt("rar", e))?;
            continue;
        }
        let (data, next) = header.read().map_err(|e| corrupt("rar", e))?;
        entries.push(DecodedEntry { rel_path, is_dir: false, declared_size, bytes: data });
        cursor = next;
    }
    Ok(entries)
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
}
