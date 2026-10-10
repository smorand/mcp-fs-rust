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
use crate::util::PosixPath;
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

/// Test-dispatch glue for `fs.extract_archive`, mirroring the production
/// handler (`McpServer::fs_extract_archive`) so the golden contract sees the
/// same schema. Hints follow `fs.write_bytes`, the closest analogue: it writes
/// files and can replace them when `overwrite` is set.
#[cfg(test)]
pub(crate) fn register(reg: &mut crate::tools::registry_support::ToolRegistry) {
    use crate::tools::registry_support::{ToolSchema, handler};
    reg.add(
        ToolSchema::new(
            "fs.extract_archive",
            "Extract an archive file in place inside the volume.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the archive file to extract.")
        .opt_str_null("destination", "Destination directory for the extracted entries.")
        .opt_bool("overwrite", false, "Overwrite existing files at the destination.")
        .opt_str_null("password", "Password for an encrypted archive.")
        .read_only(false)
        .destructive(true)
        .idempotent(false)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = crate::tools::authorize_only(&ctx, &a).await?;
            let path = a.str("path")?;
            let destination = a.opt_str("destination");
            let password = a.opt_str("password");
            extract_archive(
                &ctx.state,
                &mount,
                &ctx.person,
                &path,
                destination.as_deref(),
                a.bool_or("overwrite", false),
                password.as_deref(),
            )
            .await
        }),
    );
}

/// `fs.extract_archive(mount_id, path, destination?, overwrite?, password?)`
/// (FR-NEW-001..004, FR-NEW-016..020). The caller has already authorized
/// `mount_id` (FR-NEW-002 runs strictly before this, exactly as
/// `export_zip` assumes); `person` is the identity `charge_write` keys the
/// per-session quota counter on.
///
/// Time: O(n) over the archive's entry list for the conflict scan (one
/// `exists` probe per entry) plus the O(1) quota charge; decode is the
/// dominant cost and is already paid in the prior story. Space: O(n) for the
/// decoded entry list, no entry bytes retained past decode.
pub(crate) async fn extract_archive(
    state: &AppState,
    mount_id: &str,
    person: &str,
    path: &str,
    destination: Option<&str>,
    overwrite: bool,
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
    let (format, suffix_len) = detect_format(&normalized)?;
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
    let entries = tokio::task::spawn_blocking(move || {
        decode_archive(format, &bytes, owned_password.as_deref())
    })
    .await
    .map_err(|e| ToolError::internal(format!("archive decode task panicked: {e}")))??;

    // FR-NEW-016: caller-supplied destination normalized exactly like any
    // other `fs.*` destination parameter; otherwise the archive's own path
    // with its matched extension stripped, preserving the stem's case.
    let dest = match destination {
        Some(d) => state.safety.normalize_path(d)?,
        None => normalized[..normalized.len() - suffix_len].to_string(),
    };

    // FR-NEW-017/018: no-clobber scan in archive entry order, before any
    // write; `overwrite=true` bypasses this entirely (DEC-004).
    if !overwrite {
        for entry in &entries {
            let entry_dest = join_entry_destination(&dest, &entry.rel_path);
            if client.exists(&entry_dest).await? {
                return Err(ToolError::no_clobber(format!(
                    "'{entry_dest}' already exists; pass overwrite=true to replace it"
                )));
            }
        }
    }

    // FR-NEW-019: one charge for the archive's total declared uncompressed
    // size, after the conflict check, before any write (DEC-005). A failed
    // charge is `charge_write`'s own fail-closed behavior: nothing mutated.
    let total_declared: i64 =
        entries.iter().filter(|e| !e.is_dir).map(|e| e.declared_size as i64).sum();
    state.safety.charge_write(person, mount_id, total_declared)?;

    // FR-NEW-021: directories first (destination root, then every directory
    // entry, both in archive order), then files, each via `VolumeClient`
    // primitives directly, never `core::fs_ops::write_bytes` (DEC-006).
    //
    // `dirs_created` (FR-NEW-023) needs to know, for each directory this
    // pass creates, whether it already existed immediately before this
    // call; that one `exists` probe per directory happens before the
    // (idempotent) `makedirs` call that follows it, never twice.
    let mut dirs_created = 0u64;
    if !client.exists(&dest).await? {
        dirs_created += 1;
    }
    client.makedirs(&dest, true).await?;

    for entry in entries.iter().filter(|e| e.is_dir) {
        let entry_dest = join_entry_destination(&dest, &entry.rel_path);
        if !client.exists(&entry_dest).await? {
            dirs_created += 1;
        }
        client.makedirs(&entry_dest, true).await?;
    }

    let mut files_written = 0u64;
    let mut bytes_written = 0u64;
    for entry in entries.iter().filter(|e| !e.is_dir) {
        let entry_dest = join_entry_destination(&dest, &entry.rel_path);
        crate::core::fs_ops::ensure_parents(&client, &entry_dest).await?;
        client.write_bytes_atomic(&entry_dest, &entry.bytes).await?;
        client.touch_atime_mtime(&entry_dest);
        files_written += 1;
        bytes_written += entry.bytes.len() as u64;
    }

    // FR-NEW-025: one audit entry per call, success only, mirroring
    // `core::fs_ops::write_bytes` (`fs_ops.rs:613`). The detail string names
    // the destination and the two counters a caller would want in the
    // audit trail; it never carries `password` (FR-NEW-027).
    state.safety.record_audit(
        person,
        mount_id,
        "extract_archive",
        &normalized,
        &format!("destination={dest} files_written={files_written} bytes_written={bytes_written}"),
    );

    // FR-NEW-026: exactly these five fields, never `password` (FR-NEW-027).
    tracing::info!(
        mount_id = %mount_id,
        path = %normalized,
        destination = %dest,
        files_written = files_written,
        bytes_written = bytes_written,
        "archive extracted"
    );

    // FR-NEW-024: the response key is `destination`, never an alias.
    Ok(json!({
        "destination": dest,
        "files_written": files_written,
        "dirs_created": dirs_created,
        "bytes_written": bytes_written,
    }))
}

/// `dest` joined with an entry's archive-relative path, normalized. The
/// entry path has already passed `ensure_entry_path_safe` during decode, so
/// this join can never climb above `dest`; it still runs through
/// `PosixPath::normpath` to collapse `.`/`..` the same way every other
/// in-volume path is collapsed.
fn join_entry_destination(dest: &str, rel_path: &str) -> String {
    let joined = format!("{}/{}", dest.trim_end_matches('/'), rel_path.trim_start_matches('/'));
    PosixPath::normpath(&joined)
}

/// Open `bytes` as `format`, detect corruption, and for `zip`/`sevenz`
/// enforce the password rules (FR-NEW-007, FR-NEW-009..012). Fully decodes
/// every regular-file entry into memory during this one pass (FR-NEW-012),
/// and keeps the decoded bytes on [`DecodedEntry`] so the write pass
/// (US-0009) never decodes twice.
///
/// DRIFT-001 (resolved): the pinned `zip` 2.4.2 crate exposes entry name,
/// size and the `encrypted` flag without a password via
/// `ZipArchive::by_index_raw` (`zip-2.4.2/src/read.rs:1097-1107`) backed by
/// `ZipFileData::encrypted` (`zip-2.4.2/src/types.rs:440`); only decoding an
/// encrypted entry's bytes needs the password. `FR-NEW-009` holds as
/// written; no new requirement was needed.
fn decode_archive(
    format: ArchiveFormat,
    bytes: &[u8],
    password: Option<&str>,
) -> Result<Vec<DecodedEntry>> {
    match format {
        ArchiveFormat::Zip => decode_zip(bytes, password),
        ArchiveFormat::SevenZ => decode_sevenz(bytes, password),
        ArchiveFormat::Tar(compression) => decode_tar(compression, bytes),
    }
}

/// One archive entry, as needed past decode (US-0008/US-0009): its
/// archive-relative path (used to compute every entry's destination for the
/// no-clobber scan, FR-NEW-017, and for the write pass), whether it is a
/// directory, its declared uncompressed size (FR-NEW-009, summed over every
/// regular file for the one quota charge, FR-NEW-019), and, for a regular
/// file, its fully decoded bytes (empty `Vec` for a directory, never read).
/// Listed in archive entry order, which is what makes "first colliding path
/// in archive order" (FR-NEW-017) well defined.
#[derive(Debug, Clone)]
pub(crate) struct DecodedEntry {
    pub(crate) rel_path: String,
    pub(crate) is_dir: bool,
    pub(crate) declared_size: u64,
    pub(crate) bytes: Vec<u8>,
}

/// FR-NEW-020: an entry's decoded byte length must never exceed its
/// declared uncompressed size, independently of the quota charge (DEC-010):
/// a declared-size lie is a distinct amplification vector from a merely
/// large, honestly declared archive.
fn ensure_decoded_size_matches(rel_path: &str, declared: u64, actual: usize) -> Result<()> {
    let actual = actual as u64;
    if actual > declared {
        return Err(ToolError::invalid_argument(format!(
            "archive entry '{rel_path}' decompressed to {actual} bytes but declared {declared}"
        )));
    }
    Ok(())
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
fn decode_zip(bytes: &[u8], password: Option<&str>) -> Result<Vec<DecodedEntry>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| corrupt("zip", e))?;

    // FR-NEW-013/014/015: every entry is checked for type and path safety
    // before any entry is decoded, so a disqualifying entry anywhere in the
    // archive voids the whole call before a single byte is read. Also
    // collects the entry list (US-0008): declared size and directory-ness
    // are both already known from the raw metadata, with no password.
    let mut any_encrypted = false;
    let mut entries = Vec::with_capacity(archive.len());
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
        entries.push(DecodedEntry {
            rel_path: raw_path,
            is_dir: file.is_dir(),
            declared_size: file.size(),
            bytes: Vec::new(),
        });
    }

    if any_encrypted && password.is_none() {
        return Err(ToolError::password_required("password required to extract this archive"));
    }

    // Indexed, not iterator-driven: `by_index`/`by_index_decrypt` need the
    // numeric index itself, not just `entries[i]`.
    #[allow(clippy::needless_range_loop)]
    for i in 0..archive.len() {
        let encrypted = archive.by_index_raw(i).map_err(|e| corrupt("zip", e))?.encrypted();
        let declared = entries[i].declared_size;
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
            ensure_decoded_size_matches(&entries[i].rel_path, declared, buf.len())?;
            entries[i].bytes = buf;
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
                ensure_decoded_size_matches(&entries[i].rel_path, declared, buf.len())?;
                entries[i].bytes = buf;
            }
            Err(zip::result::ZipError::InvalidPassword) => {
                return Err(ToolError::password_required("incorrect password for this archive"));
            }
            Err(e) => return Err(corrupt("zip", e)),
        }
    }
    Ok(entries)
}

/// `sevenz`: `sevenz_rust2::Archive::read` fails outright
/// (`Error::PasswordRequired`) when the archive's own header is encrypted,
/// which is the carve-out `FR-NEW-009` already describes: that failure is
/// routed through the same password-required/incorrect messages rather than
/// treated as a listing step (per US-0001's resolution). Otherwise entries
/// are listed from the parsed header with no password, and any AES256
/// content coder gates the same password rules before a full decode.
fn decode_sevenz(bytes: &[u8], password: Option<&str>) -> Result<Vec<DecodedEntry>> {
    use sevenz_rust2::{ArchiveReader, Password};

    let probe_password = password.map(Password::from).unwrap_or_else(Password::empty);
    let archive = match sevenz_rust2::Archive::read(&mut Cursor::new(bytes), &probe_password) {
        Ok(a) => a,
        Err(sevenz_rust2::Error::PasswordRequired) => {
            return Err(password_required_message(password));
        }
        // An encrypted header decrypted with the wrong key decodes to garbage,
        // which the library can only report as a possible bad password: with a
        // password supplied that is FR-NEW-011's incorrect password, not a
        // corrupt archive.
        Err(sevenz_rust2::Error::MaybeBadPassword(_)) if password.is_some() => {
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

    // FR-NEW-014 path safety applies to every entry. Entry type rejection
    // (FR-NEW-013) is best effort per DEC-008: 7z has no universal symlink
    // representation, so only an entry whose unix extension attributes
    // positively name a non regular type is refused; anything else is an
    // ordinary file, still covered by the path check.
    //
    // `entry_error` is a side channel: the closure's error type is
    // `sevenz_rust2::Error`, which cannot carry a `ToolError` through
    // `for_each_entries`'s `?`. Without it a zip-slip, special entry or size
    // mismatch (FR-NEW-020) would fall to the generic corrupt/password
    // fallback below and surface with the wrong code.
    let mut entries = Vec::new();
    let mut entry_error: Option<ToolError> = None;
    let mut reader = ArchiveReader::from_archive(archive, Cursor::new(bytes), probe_password);
    let decode_result = reader.for_each_entries(|entry, read| {
        let refusal = ensure_entry_path_safe(entry.name()).err().or_else(|| {
            sevenz_special_type(entry).map(|label| {
                ToolError::not_supported(format!(
                    "archive entry '{}' is a {label}, which is not supported",
                    entry.name()
                ))
            })
        });
        if let Some(e) = refusal {
            entry_error = Some(e);
            return Err(sevenz_rust2::Error::Other("entry refused".into()));
        }
        let declared = entry.size();
        let rel_path = entry.name().to_string();
        if entry.is_directory() {
            entries.push(DecodedEntry {
                rel_path,
                is_dir: true,
                declared_size: declared,
                bytes: Vec::new(),
            });
            return Ok(true);
        }
        let mut buf = Vec::new();
        read.read_to_end(&mut buf)?;
        if let Err(e) = ensure_decoded_size_matches(&rel_path, declared, buf.len()) {
            entry_error = Some(e);
            return Err(sevenz_rust2::Error::Other("decoded size mismatch".into()));
        }
        entries.push(DecodedEntry { rel_path, is_dir: false, declared_size: declared, bytes: buf });
        Ok(true)
    });
    if let Some(e) = entry_error {
        return Err(e);
    }
    match decode_result {
        Ok(()) => Ok(entries),
        Err(_) if header_encrypted => Err(password_required_message(password)),
        Err(e) => Err(corrupt("7z", e)),
    }
}

/// `FR-NEW-010`/`FR-NEW-011` share one message rule across formats: no
/// password supplied is the missing-password message, any supplied
/// password (including an explicit empty string) that fails to decode is
/// the incorrect-password message.
/// 7-Zip sets this attribute bit when the high 16 bits carry a POSIX mode.
const SEVENZ_UNIX_EXTENSION: u32 = 0x8000;

/// The non regular type a 7z entry's unix extension attributes positively
/// name, or `None` when they are absent or name a regular file or directory
/// (DEC-008: what cannot be identified is treated as a regular file).
fn sevenz_special_type(entry: &sevenz_rust2::ArchiveEntry) -> Option<&'static str> {
    let attrs = entry.windows_attributes;
    if !entry.has_windows_attributes || attrs & SEVENZ_UNIX_EXTENSION == 0 {
        return None;
    }
    match (attrs >> 16) & 0o170_000 {
        0o120_000 => Some("symlink"),
        0o010_000 => Some("FIFO"),
        0o020_000 | 0o060_000 => Some("device"),
        0o140_000 => Some("socket"),
        _ => None,
    }
}

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
fn decode_tar(compression: TarCompression, bytes: &[u8]) -> Result<Vec<DecodedEntry>> {
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
    let tar_entries = tar.entries().map_err(|e| corrupt(format_name, e))?;
    let mut entries = Vec::new();
    for entry in tar_entries {
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
        let declared = entry.header().size().map_err(|e| corrupt(format_name, e))?;
        if entry_type.is_dir() {
            entries.push(DecodedEntry {
                rel_path: raw_path,
                is_dir: true,
                declared_size: declared,
                bytes: Vec::new(),
            });
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| corrupt(format_name, e))?;
        ensure_decoded_size_matches(&raw_path, declared, buf.len())?;
        entries.push(DecodedEntry {
            rel_path: raw_path,
            is_dir: false,
            declared_size: declared,
            bytes: buf,
        });
    }
    Ok(entries)
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
        extract_archive(&f.state, MOUNT, OWNER, path, None, false, None).await
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
        extract_archive(&f.state, MOUNT, OWNER, path, None, false, password).await
    }

    async fn extract_full(
        f: &Fixture,
        path: &str,
        destination: Option<&str>,
        overwrite: bool,
    ) -> Result<Value> {
        extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, None).await
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

    // ── US-0010: audit entry, tracing event, password never logged ───────

    /// Runs `f` on a dedicated single-threaded runtime while holding the
    /// capture lock, mirroring `tools::git`'s own `with_git_hosts_lock`: the
    /// capturing subscriber (`crate::logging::capture`) only records spans
    /// and events on the one OS thread that called `lock_for_test`, so the
    /// whole async body (and anything it `spawn_blocking`s back onto that
    /// same awaiting task) must run on that thread.
    fn with_capture_lock<F: std::future::Future>(f: F) -> F::Output {
        let _guard = crate::logging::capture::lock_for_test();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// FR-NEW-025: a successful call records exactly one audit entry naming
    /// the operation, carrying the destination/files_written/bytes_written
    /// in its detail, mirroring `core::fs_ops::write_bytes` (`fs_ops.rs:613`).
    #[tokio::test]
    async fn fr_new_025_success_records_one_audit_entry() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"hello")], None);
        seed_bytes(&f, "/uploads/audited.zip", &bytes).await;
        let out = extract(&f, "/uploads/audited.zip").await.unwrap();

        let log = f.state.safety.audit(OWNER, MOUNT);
        let entries: Vec<_> = log.iter().filter(|e| e.op == "extract_archive").collect();
        assert_eq!(entries.len(), 1, "exactly one audit entry");
        let detail = &entries[0].detail;
        assert!(detail.contains(out["destination"].as_str().unwrap()), "{detail}");
        assert!(detail.contains(&out["files_written"].to_string()), "{detail}");
        assert!(detail.contains(&out["bytes_written"].to_string()), "{detail}");
    }

    /// E2E-NEW-028: a failing call (wrong password) logs no line containing
    /// the literal attempted password.
    #[test]
    fn e2e_new_028_failing_call_logs_no_literal_password() {
        with_capture_lock(async {
            let f = fixture().await;
            let bytes =
                build_zip(&[("secret.txt", b"top secret")], Some("S3cr3t-Pa55-Unique-7731"));
            seed_bytes(&f, "/uploads/wrongpw.zip", &bytes).await;

            crate::logging::capture::clear();
            let _ = extract_with_password(&f, "/uploads/wrongpw.zip", Some("wrong")).await;

            for ev in crate::logging::capture::events() {
                for v in ev.fields.values() {
                    assert!(!v.contains("wrong"), "event field leaked attempted password: {v}");
                }
            }
            for sp in crate::logging::capture::spans() {
                for v in sp.fields.values() {
                    assert!(!v.contains("wrong"), "span field leaked attempted password: {v}");
                }
            }
        });
    }

    /// E2E-NEW-029: a succeeding call's log line also contains no literal
    /// substring of the correct password.
    #[test]
    fn e2e_new_029_succeeding_call_logs_no_literal_password() {
        with_capture_lock(async {
            let f = fixture().await;
            let bytes =
                build_zip(&[("secret.txt", b"top secret")], Some("S3cr3t-Pa55-Unique-7731"));
            seed_bytes(&f, "/uploads/rightpw.zip", &bytes).await;

            crate::logging::capture::clear();
            extract_with_password(&f, "/uploads/rightpw.zip", Some("S3cr3t-Pa55-Unique-7731"))
                .await
                .unwrap();

            for ev in crate::logging::capture::events() {
                for v in ev.fields.values() {
                    assert!(
                        !v.contains("S3cr3t-Pa55-Unique-7731"),
                        "event field leaked the password: {v}"
                    );
                }
            }
            for sp in crate::logging::capture::spans() {
                for v in sp.fields.values() {
                    assert!(
                        !v.contains("S3cr3t-Pa55-Unique-7731"),
                        "span field leaked the password: {v}"
                    );
                }
            }
        });
    }

    /// E2E-NEW-044: the success tracing event carries exactly the five named
    /// fields (`mount_id`, `path`, `destination`, `files_written`,
    /// `bytes_written`), no `password` key present at all; a failure (wrong
    /// password) emits no event carrying a `password` key either.
    #[test]
    fn e2e_new_044_success_event_has_exactly_five_fields_no_password_key() {
        with_capture_lock(async {
            let f = fixture().await;
            let bytes = build_zip(&[("a.txt", b"hello")], None);
            seed_bytes(&f, "/uploads/fields.zip", &bytes).await;

            crate::logging::capture::clear();
            extract(&f, "/uploads/fields.zip").await.unwrap();

            let events = crate::logging::capture::events();
            let with_all_fields = events.iter().find(|e| {
                ["mount_id", "path", "destination", "files_written", "bytes_written"]
                    .iter()
                    .all(|k| e.fields.contains_key(*k))
            });
            let event = with_all_fields.expect("no event carried the five named fields");
            let keys: std::collections::BTreeSet<_> = event.fields.keys().cloned().collect();
            let expected: std::collections::BTreeSet<_> = [
                "mount_id".to_string(),
                "path".to_string(),
                "destination".to_string(),
                "files_written".to_string(),
                "bytes_written".to_string(),
                "message".to_string(),
            ]
            .into_iter()
            .collect();
            assert_eq!(keys, expected, "event must carry exactly the five named fields");
            assert!(!event.fields.contains_key("password"));

            for ev in &events {
                assert!(!ev.fields.contains_key("password"), "no event may carry a password key");
            }
            for sp in crate::logging::capture::spans() {
                assert!(!sp.fields.contains_key("password"), "no span may carry a password key");
            }

            crate::logging::capture::clear();
            let bytes2 =
                build_zip(&[("secret.txt", b"top secret")], Some("S3cr3t-Pa55-Unique-7731"));
            seed_bytes(&f, "/uploads/fields2.zip", &bytes2).await;
            let _ = extract_with_password(&f, "/uploads/fields2.zip", Some("wrong")).await;
            for ev in crate::logging::capture::events() {
                assert!(!ev.fields.contains_key("password"), "no event may carry a password key");
            }
            for sp in crate::logging::capture::spans() {
                assert!(!sp.fields.contains_key("password"), "no span may carry a password key");
            }
        });
    }

    // ── US-0008: destination; no-clobber/overwrite; quota charge; decoded
    // size vs. declared size ───────────────────────────────────────────────

    /// E2E-NEW-010: destination collision, `overwrite` omitted (defaults
    /// false): the whole call fails, naming the colliding path, and nothing
    /// changes on disk.
    #[tokio::test]
    async fn e2e_new_010_destination_collision_overwrite_defaults_false() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"new"), ("sub/b.txt", b"new2")], None);
        seed_bytes(&f, "/uploads/report.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/uploads/report/a.txt", b"old").await.unwrap();

        let e = extract(&f, "/uploads/report.zip").await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);
        assert!(e.message.contains("/uploads/report/a.txt"), "{}", e.message);
        assert_eq!(c.read_bytes("/uploads/report/a.txt").await.unwrap(), b"old");
        assert!(!c.exists("/uploads/report/sub/b.txt").await.unwrap());
    }

    /// E2E-NEW-011: the message names the FIRST colliding path in archive
    /// entry order, not lexicographic order: `sub/b.txt` is listed first in
    /// the archive and collides, `a.txt` is listed second and also
    /// collides, but only `sub/b.txt` is named.
    #[tokio::test]
    async fn e2e_new_011_collision_names_first_colliding_path_in_archive_order() {
        let f = fixture().await;
        let bytes = build_zip(&[("sub/b.txt", b"new2"), ("a.txt", b"new")], None);
        seed_bytes(&f, "/uploads/report2.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/report2/sub", true).await.unwrap();
        c.write_bytes_atomic("/uploads/report2/sub/b.txt", b"old2").await.unwrap();
        c.write_bytes_atomic("/uploads/report2/a.txt", b"old").await.unwrap();

        let e = extract(&f, "/uploads/report2.zip").await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);
        assert!(e.message.contains("/uploads/report2/sub/b.txt"), "{}", e.message);
        assert!(!e.message.contains("/uploads/report2/a.txt"), "{}", e.message);
    }

    /// E2E-NEW-049: the conflict check also fires against a pre-existing
    /// FILE at a path the archive wants to use as a directory, not just a
    /// file-vs-file collision.
    #[tokio::test]
    async fn e2e_new_049_no_clobber_conflict_against_pre_existing_directory_entry() {
        let f = fixture().await;
        let bytes = build_zip(&[("d/", b""), ("d/x.txt", b"x")], None);
        seed_bytes(&f, "/uploads/report3.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/uploads/report3/d", b"preexisting file").await.unwrap();

        let e = extract(&f, "/uploads/report3.zip").await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);
        assert!(e.message.contains("/uploads/report3/d"), "{}", e.message);
    }

    /// FR-NEW-018: `overwrite=true` bypasses the conflict scan entirely, so
    /// the same archive that fails above succeeds once `overwrite` is set.
    /// (The write pass itself is out of scope for this story, US-0009; this
    /// only proves the conflict check is skipped, i.e. the call reaches the
    /// quota charge and returns `Ok`.)
    #[tokio::test]
    async fn overwrite_true_bypasses_the_conflict_scan() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"new")], None);
        seed_bytes(&f, "/uploads/report4.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/uploads/report4/a.txt", b"old").await.unwrap();

        extract_full(&f, "/uploads/report4.zip", None, true).await.unwrap();
    }

    /// FR-NEW-016: an explicit `destination` is normalized like any other
    /// `fs.*` destination parameter and used verbatim (modulo normalization)
    /// instead of the archive's own stripped-extension stem.
    #[tokio::test]
    async fn explicit_destination_is_used_instead_of_the_stripped_stem() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"new")], None);
        seed_bytes(&f, "/uploads/named.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/uploads/named/a.txt", b"irrelevant, different destination")
            .await
            .unwrap();

        // The default destination (`/uploads/named`) collides; an explicit,
        // disjoint destination does not, proving the explicit value, not
        // the stripped stem, is what the conflict scan actually used.
        extract_full(&f, "/uploads/named.zip", Some("/uploads/elsewhere"), false).await.unwrap();
    }

    /// E2E-NEW-020: the archive's total declared size exceeds the remaining
    /// quota, failing the whole call and leaving nothing written.
    #[tokio::test]
    async fn e2e_new_020_archive_total_declared_size_exceeds_remaining_quota() {
        let f = Fixture::with_config(|c| {
            c.safety.write_quota_bytes = 100;
        })
        .await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        let content = vec![b'x'; 10_000];
        let bytes = build_zip(&[("big.bin", &content)], None);
        c.write_bytes_atomic("/uploads/big.zip", &bytes).await.unwrap();

        let e = extract(&f, "/uploads/big.zip").await.unwrap_err();
        assert_eq!(e.code, code::WRITE_QUOTA_EXCEEDED);
        assert!(!c.exists("/uploads/big/big.bin").await.unwrap());
    }

    /// E2E-NEW-021: continuation of E2E-NEW-020, same session: a small,
    /// unrelated write within the ORIGINAL headroom still succeeds,
    /// observationally proving the failed extraction charged nothing (not
    /// by reading `SafetyManager`'s internals, by exercising the quota
    /// through a second real write).
    #[tokio::test]
    async fn e2e_new_021_after_quota_rejection_an_unrelated_small_write_still_succeeds() {
        let f = Fixture::with_config(|c| {
            c.safety.write_quota_bytes = 100;
        })
        .await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        let content = vec![b'x'; 10_000];
        let bytes = build_zip(&[("big.bin", &content)], None);
        c.write_bytes_atomic("/uploads/big.zip", &bytes).await.unwrap();
        extract(&f, "/uploads/big.zip").await.unwrap_err();

        // 50 <= 100 headroom only holds if the failed extraction charged
        // nothing against the session counter.
        crate::core::fs_ops::write_bytes(
            &c,
            &f.state.safety,
            OWNER,
            MOUNT,
            "/uploads/small.bin",
            &[b'y'; 50],
            false,
            true,
        )
        .await
        .unwrap();
    }

    /// Builds a `.zip` with one entry, stored (uncompressed) so patching the
    /// declared uncompressed-size field never has to touch the data itself.
    fn build_zip_stored(name: &str, content: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file(name, options).unwrap();
            writer.write_all(content).unwrap();
            writer.finish().unwrap();
        }
        buf
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> usize {
        haystack.windows(needle.len()).position(|w| w == needle).expect("signature not found")
    }

    /// Patches both the local file header's and the central directory
    /// record's declared uncompressed-size field to `declared`, leaving the
    /// data (and `Stored` method's `compressed_size`, which bounds the
    /// actual read per `zip-2.4.2/src/read.rs:347`) untouched: the archive
    /// still decodes its full content, which is now larger than what it
    /// declares (FR-NEW-020, E2E-NEW-026).
    fn patch_zip_declared_size(bytes: &mut [u8], declared: u32) {
        let local = find_bytes(bytes, &[0x50, 0x4b, 0x03, 0x04]);
        bytes[local + 22..local + 26].copy_from_slice(&declared.to_le_bytes());
        let central = find_bytes(bytes, &[0x50, 0x4b, 0x01, 0x02]);
        bytes[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
    }

    /// E2E-NEW-026: a crafted zip entry whose central directory and local
    /// header both declare `uncompressed_size = 1` while the entry actually
    /// decodes to 10 bytes.
    #[tokio::test]
    async fn e2e_new_026_zip_entry_decoded_size_exceeds_declared_is_invalid_argument() {
        let f = fixture().await;
        let mut bytes = build_zip_stored("mismatch.txt", b"0123456789");
        patch_zip_declared_size(&mut bytes, 1);
        seed_bytes(&f, "/uploads/mismatch.zip", &bytes).await;

        let e = extract(&f, "/uploads/mismatch.zip").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("mismatch.txt"), "{}", e.message);
        assert!(e.message.contains("declared 1"), "{}", e.message);
    }

    /// E2E-NEW-042/E2E-NEW-043 (tar/7z size mismatch): neither crate's
    /// public writer/reader API allows forging this at the archive level.
    /// tar enforces `header.size()` as a strict read boundary
    /// (`tar-0.4.46/src/archive.rs:360`, `.take(size)`), and its one escape
    /// hatch, GNU sparse extents, cross-checks the expanded length against
    /// `real_size` and errors on any discrepancy
    /// (`tar-0.4.46/src/archive.rs:530-549`), so there is no way to make the
    /// reader itself yield more bytes than the header declares. A genuine
    /// 7z-level forgery needs hand-patching the folder header's packed
    /// varint size tables, out of proportion to this P2 fixture. Both are
    /// therefore exercised directly against the one guard every decoder
    /// calls, `ensure_decoded_size_matches`, the same fallback E2E-NEW-026's
    /// own acceptance text explicitly allows ("a unit-level test that calls
    /// the internal decode-and-verify step with a mocked declared size").
    #[test]
    fn e2e_new_042_tar_decoded_size_exceeds_declared_is_invalid_argument() {
        let e = ensure_decoded_size_matches("bad.txt", 1, 10).unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("bad.txt"), "{}", e.message);
        assert!(e.message.contains("declared 1"), "{}", e.message);
    }

    #[test]
    fn e2e_new_043_sevenz_decoded_size_exceeds_declared_is_invalid_argument() {
        let e = ensure_decoded_size_matches("folder/bad.bin", 5, 20).unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("folder/bad.bin"), "{}", e.message);
        assert!(e.message.contains("declared 5"), "{}", e.message);
    }

    // ── US-0009: write pass; result counts; response shape ───────────────

    /// E2E-NEW-001: extract a `.tar.gz` with the default destination; both
    /// files land on disk and the counts/response shape are exact.
    #[tokio::test]
    async fn e2e_new_001_extract_tar_gz_default_destination() {
        let f = fixture().await;
        let bytes = build_tar_gz(&[("a.txt", b"hello"), ("sub/b.txt", b"world")]);
        seed_bytes(&f, "/uploads/report.tar.gz", &bytes).await;

        let result = extract(&f, "/uploads/report.tar.gz").await.unwrap();
        assert_eq!(result["destination"], "/uploads/report");
        assert_eq!(result["files_written"], 2);
        assert_eq!(result["dirs_created"], 1);
        assert_eq!(result["bytes_written"], 10);

        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/report/a.txt").await.unwrap(), b"hello");
        assert_eq!(c.read_bytes("/uploads/report/sub/b.txt").await.unwrap(), b"world");
    }

    /// E2E-NEW-002: each of the six supported formats extracts its single
    /// entry to disk with the default destination.
    #[tokio::test]
    async fn e2e_new_002_six_formats_extract_default_destination() {
        async fn check(f: &Fixture, path: &str, bytes: Vec<u8>, expected_dir: &str) {
            seed_bytes(f, path, &bytes).await;
            extract(f, path).await.unwrap();
            let c = f.state.stores.client(MOUNT).await.unwrap();
            assert_eq!(
                c.read_bytes(&format!("{expected_dir}/only.txt")).await.unwrap(),
                b"x",
                "{path}"
            );
        }

        let f = fixture().await;
        check(&f, "/u/f.zip", build_zip(&[("only.txt", b"x")], None), "/u/f").await;
        check(&f, "/u2/f.7z", build_sevenz(&[("only.txt", b"x")]), "/u2/f").await;
        check(&f, "/u3/f.tar", build_tar(&[("only.txt", b"x")]), "/u3/f").await;
        check(&f, "/u4/f.tgz", build_tar_gz(&[("only.txt", b"x")]), "/u4/f").await;

        // `.tb2` / `.txz`: reuse the tar builder, recompressed with the matching codec.
        let plain_tar = build_tar(&[("only.txt", b"x")]);
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(&plain_tar).unwrap();
        let bz2_bytes = encoder.finish().unwrap();
        check(&f, "/u5/f.tb2", bz2_bytes, "/u5/f").await;

        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut Cursor::new(&plain_tar), &mut xz_bytes).unwrap();
        check(&f, "/u6/f.txz", xz_bytes, "/u6/f").await;
    }

    /// E2E-NEW-003: an explicit `destination` overrides the stripped stem;
    /// the would-be default never gets created.
    #[tokio::test]
    async fn e2e_new_003_explicit_destination_override() {
        let f = fixture().await;
        let bytes = build_zip(&[("x.txt", b"y")], None);
        seed_bytes(&f, "/uploads/report.zip", &bytes).await;

        let result =
            extract_full(&f, "/uploads/report.zip", Some("/extracted"), false).await.unwrap();
        assert_eq!(result["destination"], "/extracted");

        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/extracted/x.txt").await.unwrap(), b"y");
        assert!(!c.exists("/uploads/report").await.unwrap());
    }

    /// E2E-NEW-012: a destination collision with `overwrite=true` replaces
    /// the colliding files' content.
    #[tokio::test]
    async fn e2e_new_012_overwrite_true_replaces_colliding_files() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"new"), ("sub/b.txt", b"new2")], None);
        seed_bytes(&f, "/uploads/report.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/uploads/report/a.txt", b"old").await.unwrap();

        extract_full(&f, "/uploads/report.zip", None, true).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/report/a.txt").await.unwrap(), b"new");
        assert_eq!(c.read_bytes("/uploads/report/sub/b.txt").await.unwrap(), b"new2");
    }

    /// E2E-NEW-013: `overwrite=true` against a pre-existing directory entry
    /// reuses it rather than recreating/emptying it.
    #[tokio::test]
    async fn e2e_new_013_overwrite_true_reuses_pre_existing_directory() {
        let f = fixture().await;
        let bytes = build_zip(&[("sub/", b""), ("sub/c.txt", b"c")], None);
        seed_bytes(&f, "/uploads/report5.zip", &bytes).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/report5/sub", true).await.unwrap();
        c.write_bytes_atomic("/uploads/report5/sub/other.txt", b"keep me").await.unwrap();

        extract_full(&f, "/uploads/report5.zip", None, true).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/report5/sub/other.txt").await.unwrap(), b"keep me");
        assert_eq!(c.read_bytes("/uploads/report5/sub/c.txt").await.unwrap(), b"c");
    }

    /// E2E-NEW-050: `overwrite=true` with zero actual collisions is a no-op
    /// success, proving the flag changes nothing on a genuinely clean run.
    #[tokio::test]
    async fn e2e_new_050_overwrite_true_with_no_collisions_is_plain_success() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"a"), ("b.txt", b"b")], None);
        seed_bytes(&f, "/uploads/report6.zip", &bytes).await;

        extract_full(&f, "/uploads/report6.zip", None, true).await.unwrap();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/report6/a.txt").await.unwrap(), b"a");
        assert_eq!(c.read_bytes("/uploads/report6/b.txt").await.unwrap(), b"b");
    }

    /// E2E-NEW-051: a destination override two levels deep, with no
    /// intermediate directory existing beforehand; `dirs_created` counts
    /// only the destination root, not the transitively-created parents.
    #[tokio::test]
    async fn e2e_new_051_nested_destination_counts_only_the_root_dir() {
        let f = fixture().await;
        let bytes = build_zip(&[("x.txt", b"y")], None);
        seed_bytes(&f, "/uploads/nested.zip", &bytes).await;

        let result = extract_full(&f, "/uploads/nested.zip", Some("/a/b/c"), false).await.unwrap();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/a/b/c/x.txt").await.unwrap(), b"y");
        assert!(result["dirs_created"].as_i64().unwrap() >= 1);
    }

    /// E2E-NEW-052: an AES zip with a zero-byte entry; `bytes_written == 0`,
    /// `files_written == 1`.
    #[tokio::test]
    async fn e2e_new_052_aes_zip_zero_byte_entry() {
        let f = fixture().await;
        let bytes = build_zip(&[("empty.txt", b"")], Some("p"));
        seed_bytes(&f, "/uploads/zerobyte.zip", &bytes).await;

        let result = extract_with_password(&f, "/uploads/zerobyte.zip", Some("p")).await.unwrap();
        assert_eq!(result["bytes_written"], 0);
        assert_eq!(result["files_written"], 1);
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/zerobyte/empty.txt").await.unwrap(), b"");
    }

    /// E2E-NEW-053: no pre-existing collision at all; the conflict machinery
    /// does not misfire on the happy path.
    #[tokio::test]
    async fn e2e_new_053_no_collision_succeeds_with_default_overwrite() {
        let f = fixture().await;
        let bytes = build_zip(&[("a.txt", b"new"), ("sub/b.txt", b"new2")], None);
        seed_bytes(&f, "/uploads/report7.zip", &bytes).await;

        extract_full(&f, "/uploads/report7.zip", None, false).await.unwrap();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/report7/a.txt").await.unwrap(), b"new");
        assert_eq!(c.read_bytes("/uploads/report7/sub/b.txt").await.unwrap(), b"new2");
    }

    /// E2E-NEW-054: a benign `.tar` with no escaping entries writes normally.
    #[tokio::test]
    async fn e2e_new_054_benign_tar_writes_normally() {
        let f = fixture().await;
        let bytes = build_tar(&[("good.txt", b"benign")]);
        seed_bytes(&f, "/uploads/benign.tar", &bytes).await;

        extract(&f, "/uploads/benign.tar").await.unwrap();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/benign/good.txt").await.unwrap(), b"benign");
    }

    /// E2E-NEW-055: a benign `.tar` with no symlink entries writes normally.
    #[tokio::test]
    async fn e2e_new_055_tar_with_only_regular_files_writes_normally() {
        let f = fixture().await;
        let bytes = build_tar(&[("good.txt", b"benign")]);
        seed_bytes(&f, "/uploads/benign2.tar", &bytes).await;

        extract(&f, "/uploads/benign2.tar").await.unwrap();
    }

    /// E2E-NEW-056: an uppercase `.ZIP` extension is matched case
    /// insensitively end to end, including the write pass.
    #[tokio::test]
    async fn e2e_new_056_uppercase_zip_extension_writes_normally() {
        let f = fixture().await;
        let bytes = build_zip(&[("x.txt", b"y")], None);
        seed_bytes(&f, "/uploads/REPORT.ZIP", &bytes).await;

        extract(&f, "/uploads/REPORT.ZIP").await.unwrap();
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert_eq!(c.read_bytes("/uploads/REPORT/x.txt").await.unwrap(), b"y");
    }

    /// E2E-NEW-027 (structural): `tools::archive` reuses
    /// `core::fs_ops::ensure_parents` rather than re-implementing a
    /// parent-directory walk loop.
    #[test]
    fn e2e_new_027_archive_reuses_ensure_parents_no_duplicate_loop() {
        let source = std::fs::read_to_string("src/tools/archive.rs").unwrap();
        assert!(
            source.contains("fs_ops::ensure_parents"),
            "must call core::fs_ops::ensure_parents, not reimplement it"
        );
        let needle = concat!("rfind", "('/')");
        for line in source.lines() {
            // Excludes this very assertion's own source line, which
            // necessarily names both substrings to describe what it forbids.
            if line.contains("must not re-implement") {
                continue;
            }
            assert!(
                !(line.contains(needle) && line.contains("makedirs")),
                "must not re-implement a parent-directory-walk loop: {line}"
            );
        }
    }

    /// E2E-NEW-033: the human contract lists the tool with its exact schema
    /// line, hints and return shape.
    #[test]
    fn e2e_new_033_tool_contract_txt_lists_extract_archive() {
        let txt = include_str!("../../../../TOOL_CONTRACT.txt");
        assert!(txt.contains(
            "fs.extract_archive\n  desc: Extract an archive file in place inside the volume.\n  \
             params: mount_id:string, path:string, destination:string=null, overwrite:boolean=false, \
             password:string=null\n  required: ['mount_id', 'path']\n  annotations: \
             destructiveHint=true, readOnlyHint=false, idempotentHint=false, openWorldHint=false\n"
        ));
        assert!(txt.contains("fs.extract_archive: {\"destination\": \"/uploads/report\""));
    }

    /// E2E-NEW-048: the machine checked twin carries the same tool, required
    /// fields `mount_id` and `path` only.
    #[test]
    fn e2e_new_048_golden_json_includes_extract_archive() {
        let tools = crate::tools::contract_golden::frozen_tools().expect("golden present");
        let t = tools.iter().find(|t| t["name"] == "fs.extract_archive").expect("listed");
        assert_eq!(t["inputSchema"]["required"], json!(["mount_id", "path"]));
        assert_eq!(t["annotations"]["destructiveHint"], true);
    }

    /// E2E-NEW-061: the live router serves the tool, and the frozen fs
    /// family count moved by exactly one (38 to 39).
    #[test]
    fn e2e_new_061_router_serves_extract_archive_and_fs_count_is_39() {
        let names: Vec<String> = crate::mcp::server::McpServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(names.iter().any(|n| n == "fs.extract_archive"));
        // SPEC-0019 adds fs.list_tables, 39 to 40, then fs.get_table, 41, then
        // fs.list_images and fs.get_image, 43, then fs.get_document_view, 44.
        assert_eq!(names.iter().filter(|n| n.starts_with("fs.")).count(), 44);
    }

    // ── converge round 1 (US-0013..US-0015) ─────────────────────────────────

    /// A 7z built with `sevenz-rust2`. `attrs` sets an entry's raw attributes;
    /// `password` encrypts the content (and the header unless `plain_header`).
    fn build_sevenz_ext(
        entries: &[(&str, &[u8], Option<u32>)],
        password: Option<&str>,
        plain_header: bool,
    ) -> Vec<u8> {
        use sevenz_rust2::encoder_options::AesEncoderOptions;
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderMethod};
        let mut buf = Vec::new();
        {
            let mut writer = ArchiveWriter::new(Cursor::new(&mut buf)).unwrap();
            if let Some(pw) = password {
                writer.set_content_methods(vec![
                    AesEncoderOptions::new(pw.into()).into(),
                    EncoderMethod::LZMA2.into(),
                ]);
                writer.set_encrypt_header(!plain_header);
            }
            for (name, content, attrs) in entries {
                let mut entry = ArchiveEntry::new_file(name);
                if let Some(a) = attrs {
                    entry.has_windows_attributes = true;
                    entry.windows_attributes = *a;
                }
                writer.push_archive_entry(entry, Some(Cursor::new(*content))).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    /// The standard CRC-32 (IEEE), bitwise: test fixtures only.
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

    /// One stored entry under legacy ZipCrypto (PKWARE traditional encryption),
    /// assembled by hand: the zip crate only exposes that writer crate
    /// internally, and E2E-NEW-005 needs a real legacy encrypted entry.
    fn build_zipcrypto(name: &str, content: &[u8], password: &str) -> Vec<u8> {
        fn crc_byte(crc: u32, b: u8) -> u32 {
            let mut c = crc ^ u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
            c
        }
        let mut k = [0x1234_5678u32, 0x2345_6789, 0x3456_7890];
        let update = |k: &mut [u32; 3], p: u8| {
            k[0] = crc_byte(k[0], p);
            k[1] = (k[1].wrapping_add(k[0] & 0xff)).wrapping_mul(134_775_813).wrapping_add(1);
            k[2] = crc_byte(k[2], (k[1] >> 24) as u8);
        };
        for b in password.bytes() {
            update(&mut k, b);
        }
        let crc = crc32(content);
        let mut plain = vec![0x5Au8; 11];
        plain.push((crc >> 24) as u8);
        plain.extend_from_slice(content);
        let cipher: Vec<u8> = plain
            .iter()
            .map(|&p| {
                let t = (k[2] | 2) & 0xffff;
                let c = p ^ ((t.wrapping_mul(t ^ 1) >> 8) & 0xff) as u8;
                update(&mut k, p);
                c
            })
            .collect();

        let (n, csize, usize_) = (name.len() as u16, cipher.len() as u32, content.len() as u32);
        let mut out = Vec::new();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        for v in [20u16, 1, 0, 0, 0x21] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, csize, usize_] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&cipher);

        let cd_offset = out.len() as u32;
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        for v in [20u16, 20, 1, 0, 0, 0x21] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, csize, usize_] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [n, 0, 0, 0, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        let cd_size = out.len() as u32 - cd_offset;

        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        for v in [0u16, 0, 1, 1] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    async fn read(f: &Fixture, path: &str) -> Vec<u8> {
        f.state.stores.client(MOUNT).await.unwrap().read_bytes(path).await.unwrap()
    }

    async fn exists(f: &Fixture, path: &str) -> bool {
        f.state.stores.client(MOUNT).await.unwrap().exists(path).await.unwrap()
    }

    /// E2E-NEW-004: an AES-encrypted 7z with the correct password extracts.
    #[tokio::test]
    async fn e2e_new_004_aes_sevenz_correct_password_extracts() {
        let f = fixture().await;
        let bytes = build_sevenz_ext(&[("secret.txt", b"top secret", None)], Some("pw7"), false);
        seed_bytes(&f, "/uploads/vault.7z", &bytes).await;
        let r = extract_with_password(&f, "/uploads/vault.7z", Some("pw7")).await.unwrap();
        assert_eq!(r["files_written"], 1);
        assert_eq!(read(&f, "/uploads/vault/secret.txt").await, b"top secret");
    }

    /// FR-NEW-010 on 7z, header encrypted and header plain: no password is
    /// "password required", nothing written.
    #[tokio::test]
    async fn sevenz_missing_password_is_password_required() {
        let f = fixture().await;
        for (i, plain_header) in [false, true].into_iter().enumerate() {
            let path = format!("/uploads/vault{i}.7z");
            let bytes = build_sevenz_ext(&[("s.txt", b"x", None)], Some("pw7"), plain_header);
            seed_bytes(&f, &path, &bytes).await;
            let e = extract(&f, &path).await.unwrap_err();
            assert_eq!(e.code, code::PASSWORD_REQUIRED, "plain_header={plain_header}");
            assert_eq!(e.message, "password required to extract this archive");
            assert!(!exists(&f, &format!("/uploads/vault{i}")).await);
        }
    }

    /// FR-NEW-011 on 7z, header encrypted and header plain: a wrong password is
    /// "incorrect password", nothing written.
    #[tokio::test]
    async fn sevenz_wrong_password_is_incorrect_password() {
        let f = fixture().await;
        for (i, plain_header) in [false, true].into_iter().enumerate() {
            let path = format!("/uploads/vault{i}.7z");
            let bytes = build_sevenz_ext(&[("s.txt", b"x", None)], Some("pw7"), plain_header);
            seed_bytes(&f, &path, &bytes).await;
            let e = extract_with_password(&f, &path, Some("wrong")).await.unwrap_err();
            assert_eq!(e.code, code::PASSWORD_REQUIRED, "plain_header={plain_header}");
            assert_eq!(e.message, "incorrect password for this archive");
            assert!(!exists(&f, &format!("/uploads/vault{i}")).await);
        }
    }

    /// E2E-NEW-005: AES-256 and legacy ZipCrypto zips with the right password.
    #[tokio::test]
    async fn e2e_new_005_aes_and_zipcrypto_zip_correct_password_extract() {
        let f = fixture().await;
        let aes = build_zip(&[("a.txt", b"aes body")], Some("pwz"));
        seed_bytes(&f, "/uploads/aes.zip", &aes).await;
        extract_with_password(&f, "/uploads/aes.zip", Some("pwz")).await.unwrap();
        assert_eq!(read(&f, "/uploads/aes/a.txt").await, b"aes body");

        let legacy = build_zipcrypto("l.txt", b"legacy body", "pwz");
        seed_bytes(&f, "/uploads/legacy.zip", &legacy).await;
        assert_eq!(
            extract(&f, "/uploads/legacy.zip").await.unwrap_err().code,
            code::PASSWORD_REQUIRED
        );
        extract_with_password(&f, "/uploads/legacy.zip", Some("pwz")).await.unwrap();
        assert_eq!(read(&f, "/uploads/legacy/l.txt").await, b"legacy body");
    }

    /// E2E-NEW-007: the stateless retry, missing then supplied, succeeds and
    /// the failed attempt left nothing behind to collide with.
    #[tokio::test]
    async fn e2e_new_007_retry_after_missing_password_succeeds() {
        let f = fixture().await;
        let bytes = build_zip(&[("s.txt", b"retry")], Some("pwz"));
        seed_bytes(&f, "/uploads/r.zip", &bytes).await;
        assert_eq!(extract(&f, "/uploads/r.zip").await.unwrap_err().code, code::PASSWORD_REQUIRED);
        let r = extract_with_password(&f, "/uploads/r.zip", Some("pwz")).await.unwrap();
        assert_eq!(r["files_written"], 1);
        assert_eq!(read(&f, "/uploads/r/s.txt").await, b"retry");
    }

    /// E2E-NEW-009: wrong then correct, succeeds.
    #[tokio::test]
    async fn e2e_new_009_retry_after_wrong_password_succeeds() {
        let f = fixture().await;
        let bytes = build_sevenz_ext(&[("s.txt", b"retry7", None)], Some("pw7"), false);
        seed_bytes(&f, "/uploads/r.7z", &bytes).await;
        let e = extract_with_password(&f, "/uploads/r.7z", Some("nope")).await.unwrap_err();
        assert_eq!(e.message, "incorrect password for this archive");
        extract_with_password(&f, "/uploads/r.7z", Some("pw7")).await.unwrap();
        assert_eq!(read(&f, "/uploads/r/s.txt").await, b"retry7");
    }

    /// E2E-NEW-058: an empty string is a (wrong) password, not an absent one.
    #[tokio::test]
    async fn e2e_new_058_empty_password_is_incorrect_not_required() {
        let f = fixture().await;
        let bytes = build_zip(&[("s.txt", b"x")], Some("pwz"));
        seed_bytes(&f, "/uploads/e.zip", &bytes).await;
        let e = extract_with_password(&f, "/uploads/e.zip", Some("")).await.unwrap_err();
        assert_eq!(e.code, code::PASSWORD_REQUIRED);
        assert_eq!(e.message, "incorrect password for this archive");
    }

    /// FR-NEW-013 7z clause (DEC-008): an entry whose unix extension
    /// attributes name a symlink voids the call; the benign entry before it is
    /// not written.
    #[tokio::test]
    async fn sevenz_symlink_entry_is_not_supported_and_nothing_written() {
        let f = fixture().await;
        let link = SEVENZ_UNIX_EXTENSION | (0o120_777 << 16);
        let bytes = build_sevenz_ext(
            &[("good.txt", b"benign", None), ("link", b"/etc/passwd", Some(link))],
            None,
            false,
        );
        seed_bytes(&f, "/uploads/l.7z", &bytes).await;
        let e = extract(&f, "/uploads/l.7z").await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert_eq!(e.message, "archive entry 'link' is a symlink, which is not supported");
        assert!(!exists(&f, "/uploads/l/good.txt").await);
    }

    /// DEC-008: without the unix extension flag nothing identifies the entry,
    /// so it is an ordinary file; a unix regular mode is too.
    #[tokio::test]
    async fn sevenz_entries_not_positively_special_are_regular_files() {
        let f = fixture().await;
        let regular = SEVENZ_UNIX_EXTENSION | (0o100_644 << 16);
        let bytes = build_sevenz_ext(
            &[("plain.txt", b"p", Some(0x20)), ("unix.txt", b"u", Some(regular))],
            None,
            false,
        );
        seed_bytes(&f, "/uploads/ok.7z", &bytes).await;
        extract(&f, "/uploads/ok.7z").await.unwrap();
        assert_eq!(read(&f, "/uploads/ok/plain.txt").await, b"p");
        assert_eq!(read(&f, "/uploads/ok/unix.txt").await, b"u");
    }

    /// FR-NEW-014 on 7z: an escaping entry is ERR_PATH_OUT_OF_BOUNDS (it was
    /// misreported as a corrupt archive), the benign entry is not written,
    /// and nothing lands at the target.
    #[tokio::test]
    async fn sevenz_zip_slip_entry_is_path_out_of_bounds() {
        let f = fixture().await;
        let bytes = build_sevenz_ext(
            &[("good.txt", b"benign", None), ("../../etc/passwd", b"pwned", None)],
            None,
            false,
        );
        seed_bytes(&f, "/uploads/evil.7z", &bytes).await;
        let e = extract(&f, "/uploads/evil.7z").await.unwrap_err();
        assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS, "{}", e.message);
        assert!(e.message.contains("../../etc/passwd"), "{}", e.message);
        assert!(!exists(&f, "/uploads/evil/good.txt").await);
        assert!(!exists(&f, "/etc/passwd").await);
    }

    /// FR-NEW-023: a directory entry that already exists is not counted, nor
    /// is the pre-existing destination root; only `new` is.
    #[tokio::test]
    async fn dirs_created_excludes_pre_existing_directory_entries() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/tree/sub", true).await.unwrap();
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            for dir in ["sub/", "new/"] {
                let mut h = tar::Header::new_gnu();
                h.set_path(dir).unwrap();
                h.set_entry_type(tar::EntryType::Directory);
                h.set_size(0);
                h.set_cksum();
                b.append(&h, std::io::empty()).unwrap();
            }
            for (name, body) in [("sub/a.txt", &b"a"[..]), ("new/b.txt", &b"b"[..])] {
                let mut h = tar::Header::new_gnu();
                h.set_path(name).unwrap();
                h.set_size(body.len() as u64);
                h.set_cksum();
                b.append(&h, body).unwrap();
            }
            b.finish().unwrap();
        }
        seed_bytes(&f, "/uploads/tree.tar", &tar_bytes).await;
        let r = extract_full(&f, "/uploads/tree.tar", None, true).await.unwrap();
        assert_eq!(r["dirs_created"], 1, "{r}");
        assert_eq!(r["files_written"], 2, "{r}");
    }

    /// E2E-NEW-057: repeating the failing call is stable. The retry is
    /// stateless, so the second failure equals the first and neither writes.
    #[tokio::test]
    async fn e2e_new_057_repeated_password_failure_is_identical() {
        let f = fixture().await;
        let bytes = build_zip(&[("s.txt", b"x")], Some("pwz"));
        seed_bytes(&f, "/uploads/twice.zip", &bytes).await;
        for password in [None, Some("bad")] {
            let first =
                extract_with_password(&f, "/uploads/twice.zip", password).await.unwrap_err();
            let second =
                extract_with_password(&f, "/uploads/twice.zip", password).await.unwrap_err();
            assert_eq!((first.code, first.message), (second.code, second.message));
            assert!(!exists(&f, "/uploads/twice").await);
        }
    }
}
