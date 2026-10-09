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
use std::io::{Cursor, Read as _};

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

    // CPU-bound (decompression, decryption): run off the async worker so one
    // large or hostile archive cannot stall the executor (rust skill, async
    // guidance on `spawn_blocking`).
    let bytes = client.read_bytes(&normalized).await?;
    let owned_password = password.map(ToOwned::to_owned);
    tokio::task::spawn_blocking(move || decode_archive(format, &bytes, owned_password.as_deref()))
        .await
        .map_err(|e| ToolError::internal(format!("archive decode task panicked: {e}")))??;

    // Entry safety, destination computation and the write pass land in
    // US-0007..US-0009.
    Ok(json!({}))
}

/// Open `bytes` as `format`, detect corruption, and for `zip`/`sevenz`
/// enforce the password rules (FR-NEW-007, FR-NEW-009..012). Fully decodes
/// every regular-file entry into memory during this one pass (FR-NEW-012),
/// so a later story's write pass never decodes twice; the decoded bytes
/// themselves are not needed by this story and are dropped.
///
/// DRIFT-001 (resolved): the pinned `zip` 2.4.2 crate exposes entry name,
/// size and the `encrypted` flag without a password via
/// `ZipArchive::by_index_raw` (`zip-2.4.2/src/read.rs:1097-1107`) backed by
/// `ZipFileData::encrypted` (`zip-2.4.2/src/types.rs:440`); only decoding an
/// encrypted entry's bytes needs the password. `FR-NEW-009` holds as
/// written; no new requirement was needed.
fn decode_archive(format: ArchiveFormat, bytes: &[u8], password: Option<&str>) -> Result<()> {
    match format {
        ArchiveFormat::Zip => decode_zip(bytes, password),
        ArchiveFormat::SevenZ => decode_sevenz(bytes, password),
        ArchiveFormat::Tar(compression) => decode_tar(compression, bytes),
    }
}

fn corrupt(format_name: &str, detail: impl std::fmt::Display) -> ToolError {
    ToolError::invalid_argument(format!(
        "archive is corrupt or not a valid {format_name} file: {detail}"
    ))
}

/// Rejects an entry path that is absolute or that climbs above the
/// destination root (FR-NEW-014), using the same walking technique
/// `tools::export::ensure_no_escape` uses (`export.rs:122-130`): split on
/// `/`, walk component by component, `..` is a one-level decrease, any
/// other non-empty, non-`.` component is a one-level increase, reject if
/// the running total goes below zero at any point.
///
/// Splitting also on `\` (not just `/`) covers Windows-style backslash
/// traversal (`..\\..\\windows\\system32\\evil.dll`) the same way, since a
/// POSIX destination never treats a backslash as a legitimate path
/// character worth preserving literally when it is used to spell `..`.
fn ensure_entry_path_safe(raw: &str) -> Result<()> {
    if raw.starts_with('/') {
        return Err(ToolError::path_out_of_bounds(format!(
            "archive entry path escapes the destination: '{raw}'"
        )));
    }
    let mut depth: usize = 0;
    for part in raw.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    ToolError::path_out_of_bounds(format!(
                        "archive entry path escapes the destination: '{raw}'"
                    ))
                })?;
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

/// `zip`: list every entry without a password (DRIFT-001), fail fast on a
/// missing password if any entry is encrypted (FR-NEW-010), then fully
/// decode every regular-file entry, surfacing a wrong password distinctly
/// from a general corruption error (FR-NEW-011).
///
/// DRIFT-002 (resolved): the pinned `zip` 2.4.2 crate exposes a native
/// `ZipFile::is_symlink()` (`zip-2.4.2/src/read.rs:1746-1749`), itself built
/// on `unix_mode()` (`zip-2.4.2/src/types.rs:555-562`, which reads
/// `external_attributes >> 16` for `System::Unix`). The native method is
/// used directly per US-0002's resolution note, rather than re-deriving the
/// `S_IFLNK` bit-mask by hand.
fn decode_zip(bytes: &[u8], password: Option<&str>) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| corrupt("zip", e))?;

    // FR-NEW-013/014/015: every entry is checked for type and path safety
    // before any entry is decoded, so a disqualifying entry anywhere in the
    // archive voids the whole call before a single byte is read.
    let mut any_encrypted = false;
    for i in 0..archive.len() {
        let file = archive.by_index_raw(i).map_err(|e| corrupt("zip", e))?;
        let raw_path = file.name().to_string();
        if file.is_symlink() {
            return Err(ToolError::not_supported(format!(
                "archive entry '{raw_path}' is a symlink, which is not supported"
            )));
        }
        ensure_entry_path_safe(&raw_path)?;
        if file.encrypted() {
            any_encrypted = true;
        }
    }

    if any_encrypted && password.is_none() {
        return Err(ToolError::password_required("password required to extract this archive"));
    }

    for i in 0..archive.len() {
        let encrypted = archive.by_index_raw(i).map_err(|e| corrupt("zip", e))?.encrypted();
        if !encrypted {
            // Still fully decoded, per FR-NEW-012: a non-encrypted entry can
            // still be corrupt (bad deflate stream) independently of any
            // password handling.
            let mut file = archive.by_index(i).map_err(|e| corrupt("zip", e))?;
            if file.is_dir() {
                continue;
            }
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| corrupt("zip", e))?;
            continue;
        }
        // An explicitly supplied empty string is a supplied-but-wrong
        // password, never the missing-password case above (FR-NEW-011).
        let pw = password.expect("password presence checked above for any encrypted entry");
        match archive.by_index_decrypt(i, pw.as_bytes()) {
            Ok(mut file) => {
                if file.is_dir() {
                    continue;
                }
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).map_err(|e| corrupt("zip", e))?;
            }
            Err(zip::result::ZipError::InvalidPassword) => {
                return Err(ToolError::password_required("incorrect password for this archive"));
            }
            Err(e) => return Err(corrupt("zip", e)),
        }
    }
    Ok(())
}

/// `sevenz`: `sevenz_rust2::Archive::read` fails outright
/// (`Error::PasswordRequired`) when the archive's own header is encrypted,
/// which is the carve-out `FR-NEW-009` already describes: that failure is
/// routed through the same password-required/incorrect messages rather than
/// treated as a listing step (per US-0001's resolution). Otherwise entries
/// are listed from the parsed header with no password, and any AES256
/// content coder gates the same password rules before a full decode.
fn decode_sevenz(bytes: &[u8], password: Option<&str>) -> Result<()> {
    use sevenz_rust2::{ArchiveReader, Password};

    let probe_password = password.map(Password::from).unwrap_or_else(Password::empty);
    let archive = match sevenz_rust2::Archive::read(&mut Cursor::new(bytes), &probe_password) {
        Ok(a) => a,
        Err(sevenz_rust2::Error::PasswordRequired) => {
            return Err(password_required_message(password));
        }
        Err(e) => return Err(corrupt("7z", e)),
    };

    let header_encrypted = archive.blocks.iter().any(|block| {
        block
            .coders
            .iter()
            .any(|coder| coder.encoder_method_id() == sevenz_rust2::EncoderMethod::ID_AES256_SHA256)
    });

    if header_encrypted && password.is_none() {
        return Err(ToolError::password_required("password required to extract this archive"));
    }

    // FR-NEW-014: path safety applies format-agnostically. Entry-type
    // rejection does not: per DEC-008, the 7z format has no universally
    // implemented symlink representation, so any entry this branch cannot
    // positively identify as a symlink is written as an ordinary regular
    // file (a safe default, since the path check still applies to it).
    let mut reader = ArchiveReader::from_archive(archive, Cursor::new(bytes), probe_password);
    let decode_result = reader.for_each_entries(|entry, read| {
        ensure_entry_path_safe(entry.name())
            .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
        if entry.is_directory() {
            return Ok(true);
        }
        let mut buf = Vec::new();
        read.read_to_end(&mut buf)?;
        Ok(true)
    });
    match decode_result {
        Ok(()) => Ok(()),
        Err(_) if header_encrypted => Err(password_required_message(password)),
        Err(e) => Err(corrupt("7z", e)),
    }
}

/// `FR-NEW-010`/`FR-NEW-011` share one message rule across formats: no
/// password supplied is the missing-password message, any supplied
/// password (including an explicit empty string) that fails to decode is
/// the incorrect-password message.
fn password_required_message(password: Option<&str>) -> ToolError {
    if password.is_none() {
        ToolError::password_required("password required to extract this archive")
    } else {
        ToolError::password_required("incorrect password for this archive")
    }
}

/// Tar family: no encryption exists in the tar/compression formats
/// themselves (the password gate above already refused any `password` for
/// these), so this step is pure corruption detection plus the full decode
/// `FR-NEW-012` mandates.
fn decode_tar(compression: TarCompression, bytes: &[u8]) -> Result<()> {
    let format_name = match compression {
        TarCompression::None => "tar",
        TarCompression::Gzip => "tar.gz",
        TarCompression::Bzip2 => "tar.bz2",
        TarCompression::Xz => "tar.xz",
    };
    let decompressed = match compression {
        TarCompression::None => bytes.to_vec(),
        TarCompression::Gzip => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(Cursor::new(bytes))
                .read_to_end(&mut out)
                .map_err(|e| corrupt(format_name, e))?;
            out
        }
        TarCompression::Bzip2 => {
            let mut out = Vec::new();
            bzip2::read::BzDecoder::new(Cursor::new(bytes))
                .read_to_end(&mut out)
                .map_err(|e| corrupt(format_name, e))?;
            out
        }
        TarCompression::Xz => {
            let mut out = Vec::new();
            lzma_rs::xz_decompress(&mut Cursor::new(bytes), &mut out)
                .map_err(|e| corrupt(format_name, e.to_string()))?;
            out
        }
    };

    let mut tar = tar::Archive::new(Cursor::new(decompressed));
    let entries = tar.entries().map_err(|e| corrupt(format_name, e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| corrupt(format_name, e))?;
        let entry_type = entry.header().entry_type();
        let raw_path =
            entry.path().map_err(|e| corrupt(format_name, e))?.to_string_lossy().into_owned();
        // FR-NEW-013: reject anything other than a regular file or a
        // directory, before touching the path check or reading content, so
        // a symlink entry is named with the right word even when its own
        // path happens to be benign.
        if let Some(label) = tar_entry_type_label(entry_type) {
            return Err(ToolError::not_supported(format!(
                "archive entry '{raw_path}' is a {label}, which is not supported"
            )));
        }
        ensure_entry_path_safe(&raw_path)?;
        if entry_type.is_dir() {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| corrupt(format_name, e))?;
    }
    Ok(())
}

/// Names `t` for the `ERR_NOT_SUPPORTED` message when it is neither a
/// regular file nor a directory (FR-NEW-013); `None` for the two accepted
/// types.
fn tar_entry_type_label(t: tar::EntryType) -> Option<&'static str> {
    use tar::EntryType;
    match t {
        EntryType::Regular | EntryType::Directory => None,
        EntryType::Symlink => Some("symlink"),
        EntryType::Link => Some("hardlink"),
        EntryType::Fifo => Some("FIFO"),
        EntryType::Char | EntryType::Block => Some("device"),
        _ => Some("special file"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::admin::test_support::Fixture;
    use std::io::Write as _;

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

    // ── fixture builders: every archive used below is built in-process with
    // the same crates the decoder uses, never a checked-in binary ─────────

    fn build_zip(entries: &[(&str, &[u8])], aes_password: Option<&str>) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, content) in entries {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                let options = match aes_password {
                    Some(pw) => options.with_aes_encryption(zip::AesMode::Aes256, pw),
                    None => options,
                };
                writer.start_file(*name, options).unwrap();
                writer.write_all(content).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    fn build_sevenz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let mut buf = Vec::new();
        {
            let mut writer = ArchiveWriter::new(Cursor::new(&mut buf)).unwrap();
            for (name, content) in entries {
                writer
                    .push_archive_entry(ArchiveEntry::new_file(name), Some(Cursor::new(*content)))
                    .unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_path(name).unwrap();
                header.set_size(content.len() as u64);
                header.set_cksum();
                builder.append(&header, *content).unwrap();
            }
            builder.finish().unwrap();
        }
        let mut gz_bytes = Vec::new();
        {
            let mut encoder =
                flate2::write::GzEncoder::new(&mut gz_bytes, flate2::Compression::default());
            encoder.write_all(&tar_bytes).unwrap();
            encoder.finish().unwrap();
        }
        gz_bytes
    }

    async fn seed_bytes(f: &Fixture, path: &str, bytes: &[u8]) {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic(path, bytes).await.unwrap();
    }

    /// DRIFT-001 (resolved): the pinned `zip` crate must expose entry name,
    /// size and the `encrypted` flag without a password. Verified against the
    /// real crate, not a mock: build an AES-encrypted zip, open it with
    /// `ZipArchive::new` and no password, and read every entry's metadata
    /// through `by_index_raw`.
    #[test]
    fn drift_001_zip_entry_metadata_readable_without_password() {
        let bytes = build_zip(&[("secret.txt", b"top secret")], Some("correct horse"));
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 1);
        let file = archive.by_index_raw(0).unwrap();
        assert_eq!(file.name(), "secret.txt");
        assert_eq!(file.size(), 10);
        assert!(file.encrypted());
    }

    /// E2E-NEW-023: a `.zip` whose bytes are not a zip file at all.
    #[tokio::test]
    async fn e2e_new_023_corrupt_zip_is_invalid_argument() {
        let f = fixture().await;
        seed_bytes(&f, "/uploads/bad.zip", b"not a zip file at all").await;
        let e = extract(&f, "/uploads/bad.zip").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("corrupt"), "{}", e.message);
    }

    /// E2E-NEW-036: a `.7z` whose bytes are not a 7z file at all.
    #[tokio::test]
    async fn e2e_new_036_corrupt_sevenz_is_invalid_argument() {
        let f = fixture().await;
        seed_bytes(&f, "/uploads/bad.7z", b"not a sevenz file at all").await;
        let e = extract(&f, "/uploads/bad.7z").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("corrupt"), "{}", e.message);
    }

    /// E2E-NEW-037: a `.tar.gz` whose gzip magic bytes are zeroed out.
    #[tokio::test]
    async fn e2e_new_037_corrupt_tar_gz_is_invalid_argument() {
        let f = fixture().await;
        let mut bytes = build_tar_gz(&[("a.txt", b"hello")]);
        bytes[0] = 0;
        bytes[1] = 0;
        seed_bytes(&f, "/uploads/bad.tar.gz", &bytes).await;
        let e = extract(&f, "/uploads/bad.tar.gz").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("corrupt"), "{}", e.message);
    }

    /// E2E-NEW-006: an AES-encrypted zip with no `password` supplied.
    #[tokio::test]
    async fn e2e_new_006_zip_missing_password_is_password_required() {
        let f = fixture().await;
        let bytes = build_zip(&[("secret.txt", b"top secret")], Some("correct horse"));
        seed_bytes(&f, "/uploads/secret.zip", &bytes).await;
        let e = extract(&f, "/uploads/secret.zip").await.unwrap_err();
        assert_eq!(e.code, code::PASSWORD_REQUIRED);
        assert!(e.message.contains("password required"), "{}", e.message);
    }

    /// E2E-NEW-057: an AES-encrypted zip, `password` supplied but wrong.
    #[tokio::test]
    async fn e2e_new_057_zip_incorrect_password_is_password_required() {
        let f = fixture().await;
        let bytes = build_zip(&[("secret.txt", b"top secret")], Some("correct horse"));
        seed_bytes(&f, "/uploads/secret.zip", &bytes).await;
        let e = extract_with_password(&f, "/uploads/secret.zip", Some("wrong guess"))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::PASSWORD_REQUIRED);
        assert!(e.message.contains("incorrect password"), "{}", e.message);
    }

    /// E2E-NEW-008/E2E-NEW-058: an AES-encrypted zip, `password` supplied and
    /// correct, succeeds this story's decode-only pass.
    #[tokio::test]
    async fn e2e_new_008_zip_correct_password_decodes() {
        let f = fixture().await;
        let bytes = build_zip(&[("secret.txt", b"top secret")], Some("correct horse"));
        seed_bytes(&f, "/uploads/secret.zip", &bytes).await;
        extract_with_password(&f, "/uploads/secret.zip", Some("correct horse")).await.unwrap();
    }

    /// A non-encrypted zip needs no password at all.
    #[tokio::test]
    async fn plain_zip_decodes_without_password() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"hello")], None);
        seed_bytes(&f, "/uploads/plain.zip", &bytes).await;
        extract(&f, "/uploads/plain.zip").await.unwrap();
    }

    /// A non-encrypted 7z needs no password and decodes cleanly.
    #[tokio::test]
    async fn plain_sevenz_decodes_without_password() {
        let f = fixture().await;
        let bytes = build_sevenz(&[("a.txt", b"hello")]);
        seed_bytes(&f, "/uploads/plain.7z", &bytes).await;
        extract(&f, "/uploads/plain.7z").await.unwrap();
    }

    /// A clean `.tar.gz` decodes without error.
    #[tokio::test]
    async fn plain_tar_gz_decodes() {
        let f = fixture().await;
        let bytes = build_tar_gz(&[("a.txt", b"hello")]);
        seed_bytes(&f, "/uploads/plain2.tar.gz", &bytes).await;
        extract(&f, "/uploads/plain2.tar.gz").await.unwrap();
    }

    // ── US-0007: symlink/hardlink/zip-slip rejection ──────────────────────

    /// Writes `name` into `header`'s raw GNU name field, bypassing
    /// `Header::set_path`'s validation (which rejects a `..` component):
    /// the fixtures below need to actually store a traversal path on disk,
    /// which is exactly the attack `ensure_entry_path_safe` must catch, so
    /// the test build step cannot itself refuse to write it.
    fn set_raw_tar_name(header: &mut tar::Header, name: &str) {
        let gnu = header.as_gnu_mut().expect("gnu header");
        gnu.name = [0u8; 100];
        gnu.name[..name.len()].copy_from_slice(name.as_bytes());
    }

    /// Plain (uncompressed) tar with regular-file entries only. Names may
    /// contain `..` (raw name field, bypassing `set_path`'s validation).
    fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                set_raw_tar_name(&mut header, name);
                header.set_size(content.len() as u64);
                header.set_cksum();
                builder.append(&header, *content).unwrap();
            }
            builder.finish().unwrap();
        }
        tar_bytes
    }

    /// A plain tar with one benign entry plus one entry of `entry_type`
    /// named `special_name`, optionally carrying a link target.
    fn build_tar_with_special_entry(
        good_name: &str,
        good_content: &[u8],
        special_name: &str,
        entry_type: tar::EntryType,
        link_target: Option<&str>,
    ) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut good = tar::Header::new_gnu();
            set_raw_tar_name(&mut good, good_name);
            good.set_size(good_content.len() as u64);
            good.set_cksum();
            builder.append(&good, good_content).unwrap();

            let mut special = tar::Header::new_gnu();
            set_raw_tar_name(&mut special, special_name);
            special.set_entry_type(entry_type);
            special.set_size(0);
            if let Some(target) = link_target {
                special.set_link_name(target).unwrap();
            }
            special.set_cksum();
            builder.append(&special, std::io::empty()).unwrap();

            builder.finish().unwrap();
        }
        tar_bytes
    }

    /// A zip with a benign entry plus one entry named exactly `special_name`
    /// (no sanitization: the raw name is stored as given). `special_as_symlink`
    /// selects `ZipWriter::add_symlink` (native symlink entry, DRIFT-002) over
    /// a plain file entry: `unix_permissions` alone cannot do this, since it
    /// masks its argument with `& 0o777` and discards the `S_IFLNK` type bit
    /// (`zip-2.4.2/src/write.rs:472-476`).
    fn build_zip_with_special_entry(
        good_name: &str,
        good_content: &[u8],
        special_name: &str,
        special_content: &[u8],
        special_as_symlink: bool,
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let plain = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file(good_name, plain).unwrap();
            writer.write_all(good_content).unwrap();

            let special_options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            if special_as_symlink {
                let target = String::from_utf8_lossy(special_content).into_owned();
                writer.add_symlink(special_name, target, special_options).unwrap();
            } else {
                writer.start_file(special_name, special_options).unwrap();
                writer.write_all(special_content).unwrap();
            }

            writer.finish().unwrap();
        }
        buf
    }

    /// E2E-NEW-014: a `.tar` with one zip-slip entry and one benign entry.
    #[tokio::test]
    async fn e2e_new_014_tar_zip_slip_entry_is_path_out_of_bounds() {
        let f = fixture().await;
        let bytes = build_tar(&[("good.txt", b"benign"), ("../../etc/passwd", b"pwned")]);
        seed_bytes(&f, "/uploads/evil.tar", &bytes).await;
        let e = extract(&f, "/uploads/evil.tar").await.unwrap_err();
        assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS);
        assert!(e.message.contains("../../etc/passwd"), "{}", e.message);
    }

    /// E2E-NEW-015: same archive, the benign entry's content never appears
    /// anywhere on disk after the failure (verified through a different
    /// channel than the tool's own response: `client.exists`).
    #[tokio::test]
    async fn e2e_new_015_tar_zip_slip_leaves_nothing_written() {
        let f = fixture().await;
        let bytes = build_tar(&[("good.txt", b"benign"), ("../../etc/passwd", b"pwned")]);
        seed_bytes(&f, "/uploads/evil.tar", &bytes).await;
        extract(&f, "/uploads/evil.tar").await.unwrap_err();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert!(!c.exists("/uploads/evil/good.txt").await.unwrap());
        assert!(!c.exists("/etc/passwd").await.unwrap());
    }

    /// E2E-NEW-040: a `.zip` entry whose stored name is absolute.
    #[tokio::test]
    async fn e2e_new_040_zip_absolute_entry_is_path_out_of_bounds() {
        let f = fixture().await;
        let bytes =
            build_zip_with_special_entry("good.txt", b"benign", "/etc/passwd", b"pwned", false);
        seed_bytes(&f, "/uploads/evil.zip", &bytes).await;
        let e = extract(&f, "/uploads/evil.zip").await.unwrap_err();
        assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS);
        assert!(e.message.contains("/etc/passwd"), "{}", e.message);
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert!(!c.exists("/uploads/evil/good.txt").await.unwrap());
    }

    /// E2E-NEW-041: a `.zip` entry with a Windows-style backslash traversal
    /// name, mirroring `tools::export::ensure_no_escape`'s own windows-style
    /// test case.
    #[tokio::test]
    async fn e2e_new_041_zip_backslash_traversal_entry_is_path_out_of_bounds() {
        let f = fixture().await;
        let bytes = build_zip_with_special_entry(
            "good.txt",
            b"benign",
            "..\\..\\windows\\system32\\evil.dll",
            b"pwned",
            false,
        );
        seed_bytes(&f, "/uploads/evil2.zip", &bytes).await;
        let e = extract(&f, "/uploads/evil2.zip").await.unwrap_err();
        assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS);
    }

    /// E2E-NEW-016: a `.tar` with a symlink entry and one benign entry.
    #[tokio::test]
    async fn e2e_new_016_tar_symlink_entry_is_not_supported() {
        let f = fixture().await;
        let bytes = build_tar_with_special_entry(
            "good.txt",
            b"benign",
            "link",
            tar::EntryType::Symlink,
            Some("/etc/passwd"),
        );
        seed_bytes(&f, "/uploads/evil3.tar", &bytes).await;
        let e = extract(&f, "/uploads/evil3.tar").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(e.message.contains("link"), "{}", e.message);
        assert!(e.message.contains("symlink"), "{}", e.message);
    }

    /// E2E-NEW-017: a `.tar` hardlink entry; the message names a hardlink,
    /// not the word "symlink".
    #[tokio::test]
    async fn e2e_new_017_tar_hardlink_entry_is_not_supported() {
        let f = fixture().await;
        let bytes = build_tar_with_special_entry(
            "good.txt",
            b"benign",
            "link",
            tar::EntryType::Link,
            Some("good.txt"),
        );
        seed_bytes(&f, "/uploads/evil4.tar", &bytes).await;
        let e = extract(&f, "/uploads/evil4.tar").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(e.message.contains("link"), "{}", e.message);
        assert!(!e.message.contains("symlink"), "{}", e.message);
    }

    /// E2E-NEW-018: a zip entry with Unix mode `S_IFLNK` (DRIFT-002). Red
    /// before the fix: `zip` crate's native `is_symlink()` is read directly,
    /// no check existed before this story.
    #[tokio::test]
    async fn e2e_new_018_zip_symlink_entry_is_not_supported() {
        let f = fixture().await;
        let bytes =
            build_zip_with_special_entry("good.txt", b"benign", "link", b"/etc/passwd", true);
        seed_bytes(&f, "/uploads/evil5.zip", &bytes).await;
        let e = extract(&f, "/uploads/evil5.zip").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(e.message.contains("link"), "{}", e.message);
    }

    /// E2E-NEW-019: symlink-carrying tar archive leaves the benign entry's
    /// content unwritten anywhere, same verification channel as
    /// E2E-NEW-015.
    #[tokio::test]
    async fn e2e_new_019_tar_symlink_leaves_nothing_written() {
        let f = fixture().await;
        let bytes = build_tar_with_special_entry(
            "good.txt",
            b"benign",
            "link",
            tar::EntryType::Symlink,
            Some("/etc/passwd"),
        );
        seed_bytes(&f, "/uploads/evil6.tar", &bytes).await;
        extract(&f, "/uploads/evil6.tar").await.unwrap_err();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert!(!c.exists("/uploads/evil6/good.txt").await.unwrap());
    }

    /// DRIFT-002 (resolved): the pinned `zip` 2.4.2 crate exposes a native
    /// `is_symlink()` on `ZipFile`, readable through `by_index_raw` with no
    /// password, confirmed against the real crate (not a mock).
    #[test]
    fn drift_002_zip_entry_is_symlink_readable_via_native_method() {
        let bytes =
            build_zip_with_special_entry("good.txt", b"benign", "link", b"/etc/passwd", true);
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let link_index = (0..archive.len())
            .find(|&i| archive.by_index_raw(i).unwrap().name() == "link")
            .unwrap();
        assert!(archive.by_index_raw(link_index).unwrap().is_symlink());
        let good_index = (0..archive.len())
            .find(|&i| archive.by_index_raw(i).unwrap().name() == "good.txt")
            .unwrap();
        assert!(!archive.by_index_raw(good_index).unwrap().is_symlink());
    }

    /// FR-NEW-014's own walking technique, exercised directly: ordinary
    /// relative paths and in-bounds `..` are accepted, an absolute path or a
    /// net-negative walk (slash or backslash separated) is rejected.
    #[test]
    fn ensure_entry_path_safe_accepts_in_bounds_rejects_escapes() {
        for ok in ["good.txt", "a/b/../c.txt", "./a.txt", "a/.."] {
            ensure_entry_path_safe(ok).unwrap();
        }
        for bad in ["/etc/passwd", "../../etc/passwd", "..\\..\\windows\\system32\\evil.dll"] {
            ensure_entry_path_safe(bad).unwrap_err();
        }
    }
}
