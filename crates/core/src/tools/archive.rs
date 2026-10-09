//! `fs.extract_archive` (SPEC-0015): extract an archive file in place inside
//! the volume.
//!
//! Typed function only: the `#[tool]` method in `mcp::server`
//! (`McpServer::fs_extract_archive`) and, once US-0011 lands, the REST route
//! both authorize, then call [`extract_archive`], so the two doors share one
//! implementation (`tools::export::export_zip` is the structural template).
//!
//! The function normalizes `path`, rejects a missing node and a directory
//! node, then detects the format from the filename alone and refuses a
//! password against a tar family archive before any byte is read. Decode and
//! the real result shape land in later stories (US-0006 onward).

use crate::errors::{Result, ToolError};
use crate::state::AppState;
use serde_json::{Value, json};

/// Compression wrapped around a tar stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TarCompression {
    None,
    Gzip,
    Bzip2,
    Xz,
}

/// An archive format, decided from the filename extension only (FR-NEW-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveFormat {
    Zip,
    SevenZ,
    Tar(TarCompression),
}

/// Suffix table, matched in order: every compound tar suffix sits before the
/// plain `.tar` so `x.tar.gz` can never be read as an uncompressed tar.
const SUFFIXES: &[(&str, ArchiveFormat)] = &[
    (".tar.gz", ArchiveFormat::Tar(TarCompression::Gzip)),
    (".tgz", ArchiveFormat::Tar(TarCompression::Gzip)),
    (".tar.bz2", ArchiveFormat::Tar(TarCompression::Bzip2)),
    (".tb2", ArchiveFormat::Tar(TarCompression::Bzip2)),
    (".tar.xz", ArchiveFormat::Tar(TarCompression::Xz)),
    (".txz", ArchiveFormat::Tar(TarCompression::Xz)),
    (".tar", ArchiveFormat::Tar(TarCompression::None)),
    (".zip", ArchiveFormat::Zip),
    (".7z", ArchiveFormat::SevenZ),
];

/// Detect the format of `path` from its filename, case insensitively
/// (FR-NEW-005). Returns the format and the matched suffix length in bytes,
/// which the default destination later strips (FR-NEW-016). An unmatched
/// name fails with `ERR_NOT_SUPPORTED` naming the last extension (FR-NEW-006).
pub(crate) fn detect_format(path: &str) -> Result<(ArchiveFormat, usize)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix))
        .map(|&(suffix, fmt)| (fmt, suffix.len()))
        .ok_or_else(|| {
            let ext = name.rfind('.').map_or("", |i| &name[i..]);
            ToolError::not_supported(format!("unsupported archive extension: '{ext}'"))
        })
}

/// `fs.extract_archive(mount_id, path, destination?, overwrite?, password?)`
/// (FR-NEW-001..004). The caller has already authorized `mount_id`
/// (FR-NEW-002 runs strictly before this, exactly as `export_zip` assumes).
///
/// Time: O(1) node lookups in this story (format detection and the actual
/// extraction walk are deferred to later stories). Space: O(1).
pub(crate) async fn extract_archive(
    state: &AppState,
    mount_id: &str,
    path: &str,
    _destination: Option<&str>,
    _overwrite: bool,
    password: Option<&str>,
) -> Result<Value> {
    let normalized = state.safety.normalize_path(path)?;

    let client = state.stores.client(mount_id).await?;
    let node = client
        .meta
        .get(&normalized)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("'{path}' not found")))?;
    if node.is_dir() {
        return Err(ToolError::invalid_argument(format!(
            "'{path}' is a directory, not an archive file"
        )));
    }

    // Decided on the name alone: a tar stream carries no encryption, so a
    // password there is a caller mistake, refused before any byte is read
    // (FR-NEW-008).
    let (format, _suffix_len) = detect_format(&normalized)?;
    if matches!(format, ArchiveFormat::Tar(_)) && password.is_some() {
        return Err(ToolError::invalid_argument(
            "password is not applicable to this archive format",
        ));
    }

    // Decode lands in US-0006.
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::admin::test_support::Fixture;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "proj";

    async fn fixture() -> Fixture {
        let f = Fixture::with_config(|_| {}).await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/adir", true).await.unwrap();
        c.makedirs("/uploads/a/b", true).await.unwrap();
        f
    }

    async fn extract(f: &Fixture, path: &str) -> Result<Value> {
        extract_archive(&f.state, MOUNT, path, None, false, None).await
    }

    /// `path` points at a file that does not exist: `ERR_NOT_FOUND`.
    #[tokio::test]
    async fn missing_archive_is_not_found() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/missing.zip").await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(e.message.contains("/uploads/missing.zip"), "{}", e.message);
    }

    /// E2E-NEW-025: `path` points at a directory, not a file.
    #[tokio::test]
    async fn e2e_new_025_directory_path_is_invalid_argument() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/adir").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("'/uploads/adir' is a directory"), "{}", e.message);
    }

    /// E2E-NEW-059: `path` points at a directory nested two levels deep.
    #[tokio::test]
    async fn e2e_new_059_nested_directory_path_is_invalid_argument() {
        let f = fixture().await;
        let e = extract(&f, "/uploads/a/b").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("'/uploads/a/b' is a directory"), "{}", e.message);
    }

    /// E2E-NEW-060: `path` is `/` itself, rejected the same way, not special
    /// cased.
    #[tokio::test]
    async fn e2e_new_060_volume_root_is_invalid_argument_like_any_directory() {
        let f = fixture().await;
        let e = extract(&f, "/").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("'/' is a directory"), "{}", e.message);
    }

    async fn extract_with_password(
        f: &Fixture,
        path: &str,
        password: Option<&str>,
    ) -> Result<Value> {
        extract_archive(&f.state, MOUNT, path, None, false, password).await
    }

    async fn seed_file(f: &Fixture, path: &str) {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic(path, b"anything").await.unwrap();
    }

    /// E2E-NEW-022: `.rar` is not in the supported mapping.
    #[tokio::test]
    async fn e2e_new_022_rar_extension_is_not_supported() {
        let f = fixture().await;
        seed_file(&f, "/uploads/data.rar").await;
        let e = extract(&f, "/uploads/data.rar").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(e.message.contains(".rar"), "{}", e.message);
    }

    /// E2E-NEW-034: bare `.gz` (not `.tar.gz`) is not in the mapping.
    #[tokio::test]
    async fn e2e_new_034_bare_gz_extension_is_not_supported() {
        let f = fixture().await;
        seed_file(&f, "/uploads/data.gz").await;
        let e = extract(&f, "/uploads/data.gz").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(e.message.contains(".gz"), "{}", e.message);
    }

    /// E2E-NEW-035: `.zip.001` split-archive-style name is not in the mapping.
    #[tokio::test]
    async fn e2e_new_035_zip_001_extension_is_not_supported() {
        let f = fixture().await;
        seed_file(&f, "/uploads/parts.zip.001").await;
        let e = extract(&f, "/uploads/parts.zip.001").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(e.message.contains(".001"), "{}", e.message);
    }

    /// E2E-NEW-024: password supplied against a `.tar.gz` archive.
    #[tokio::test]
    async fn e2e_new_024_password_against_tar_gz_is_invalid_argument() {
        let f = fixture().await;
        seed_file(&f, "/uploads/plain.tar.gz").await;
        let e =
            extract_with_password(&f, "/uploads/plain.tar.gz", Some("anything")).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "password is not applicable to this archive format");
    }

    /// E2E-NEW-038: password supplied against a plain `.tar`.
    #[tokio::test]
    async fn e2e_new_038_password_against_plain_tar_is_invalid_argument() {
        let f = fixture().await;
        seed_file(&f, "/uploads/plain.tar").await;
        let e = extract_with_password(&f, "/uploads/plain.tar", Some("x")).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "password is not applicable to this archive format");
    }

    /// E2E-NEW-039: password supplied against a `.tar.bz2`.
    #[tokio::test]
    async fn e2e_new_039_password_against_tar_bz2_is_invalid_argument() {
        let f = fixture().await;
        seed_file(&f, "/uploads/plain.tar.bz2").await;
        let e = extract_with_password(&f, "/uploads/plain.tar.bz2", Some("x")).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "password is not applicable to this archive format");
    }

    /// Password against `.tar.xz` is rejected too (fourth tar variant, not
    /// separately enumerated as an E2E id but covered by FR-NEW-008's "all
    /// four tar variants uniformly").
    #[tokio::test]
    async fn password_against_tar_xz_is_invalid_argument() {
        let f = fixture().await;
        seed_file(&f, "/uploads/plain.tar.xz").await;
        let e = extract_with_password(&f, "/uploads/plain.tar.xz", Some("x")).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "password is not applicable to this archive format");
    }

    /// FR-NEW-005 mapping, every suffix, case insensitive, compound before
    /// plain `.tar`, with the suffix length the destination stem needs.
    #[test]
    fn detect_format_maps_every_suffix() {
        use ArchiveFormat::{SevenZ, Tar, Zip};
        use TarCompression::{Bzip2, Gzip, None as Plain, Xz};
        for (path, fmt, len) in [
            ("/a/x.zip", Zip, 4),
            ("/a/x.7z", SevenZ, 3),
            ("/a/x.tar.gz", Tar(Gzip), 7),
            ("/a/x.tgz", Tar(Gzip), 4),
            ("/a/x.tar.bz2", Tar(Bzip2), 8),
            ("/a/x.tb2", Tar(Bzip2), 4),
            ("/a/x.tar.xz", Tar(Xz), 7),
            ("/a/x.txz", Tar(Xz), 4),
            ("/a/x.tar", Tar(Plain), 4),
            ("/a/X.TAR.GZ", Tar(Gzip), 7),
            ("/a/Data.Zip", Zip, 4),
        ] {
            assert_eq!(detect_format(path).unwrap(), (fmt, len), "{path}");
        }
    }

    /// A name with no dot names an empty extension.
    #[test]
    fn detect_format_rejects_dotless_names() {
        let e = detect_format("/a/README").unwrap_err();
        assert_eq!(e.message, "unsupported archive extension: ''");
    }

    /// Compound suffix `.tar.gz` must not be mistaken for plain `.tar`
    /// (otherwise `.tar.gz` password gate would still work by accident, but
    /// a wrong format for later stories). Verified indirectly: `.TAR.GZ`
    /// case-insensitively matches and is still gated as tar.
    #[tokio::test]
    async fn uppercase_tar_gz_is_matched_case_insensitively() {
        let f = fixture().await;
        seed_file(&f, "/uploads/PLAIN.TAR.GZ").await;
        let e = extract_with_password(&f, "/uploads/PLAIN.TAR.GZ", Some("x")).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "password is not applicable to this archive format");
    }
}
