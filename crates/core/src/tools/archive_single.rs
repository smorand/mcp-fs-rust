//! Single file compression formats of `fs.extract_archive` (SPEC-0081): a
//! bare `.gz` holds one file, so it has no entry list and no folder named
//! like the archive.

use crate::errors::{Result, ToolError};
use crate::quota;
use crate::state::AppState;
use crate::storage::volume::VolumeClient;
use crate::tools::archive::{corrupt, finish_extraction};
use serde_json::Value;
use std::io::Read;

/// Read chunk of the decode loop.
const CHUNK: usize = 65_536;

/// A compression wrapped around exactly one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SingleFormat {
    Gzip,
    Bzip2,
    Xz,
}

impl SingleFormat {
    /// Name used in the corrupt archive message.
    fn label(self) -> &'static str {
        match self {
            Self::Gzip => "gz",
            Self::Bzip2 => "bz2",
            Self::Xz => "xz",
        }
    }
}

/// What a bounded read produced.
#[derive(Debug)]
pub(crate) struct CappedRead {
    pub(crate) bytes: Vec<u8>,
    /// Largest number of output bytes held at once, at most the cap.
    pub(crate) high_water: u64,
}

/// Why a bounded read stopped early.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The output would have passed the cap; `high_water` is the most bytes held.
    CapExceeded {
        high_water: u64,
    },
    Io(std::io::Error),
    /// The decoder could not be created: a fault of the server, not of the file.
    Init(std::io::Error),
}

#[cfg(test)]
thread_local! {
    /// Largest buffer the last decode of this thread held, so a test can
    /// measure the decoder inside a whole extraction.
    pub(crate) static LAST_HIGH_WATER: std::cell::Cell<Option<u64>> =
        const { std::cell::Cell::new(None) };
}

/// Decoder over `bytes`. `MultiGzDecoder` reads every gzip member back to back
/// and fails on bytes that are not a member, where `GzDecoder` would stop after
/// the first member and ignore the rest. `MultiBzDecoder` does the same for
/// bzip2: it decodes every stream and fails on bytes that are not another
/// stream, where `BzDecoder` would stop after the first and ignore the rest.
/// The xz decoder is built from an explicit xz stream decoder, not
/// `new_multi_decoder`: that one is an auto decoder that also accepts a legacy
/// `.lzma` file and decodes a check type it cannot verify without saying so.
/// `CONCATENATED` makes liblzma read every stream, accept zero padding of a
/// multiple of 4 bytes between and after them, and refuse any other trailing byte.
/// Fallible because liblzma can fail to allocate its state.
fn decoder(format: SingleFormat, bytes: &[u8]) -> std::io::Result<Box<dyn Read + '_>> {
    Ok(match format {
        SingleFormat::Gzip => Box::new(flate2::read::MultiGzDecoder::new(bytes)),
        SingleFormat::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(bytes)),
        SingleFormat::Xz => {
            let stream = liblzma::stream::Stream::new_stream_decoder(
                u64::MAX,
                liblzma::stream::CONCATENATED | liblzma::stream::TELL_UNSUPPORTED_CHECK,
            )?;
            Box::new(liblzma::read::XzDecoder::new_stream(bytes, stream))
        }
    })
}

/// Read `reader` to its end in `CHUNK` pieces, stopping with `CapExceeded`
/// once the output would pass `cap` bytes (equal is accepted); at most `cap`
/// bytes are held, plus the fixed chunk. Time O(n) in the output read, memory O(min(n, cap)):
/// the output is held once and the chunk buffer is fixed.
fn read_capped(mut reader: impl Read, cap: u64) -> std::result::Result<CappedRead, ReadError> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let n = reader.read(&mut chunk).map_err(ReadError::Io)?;
        if n == 0 {
            break;
        }
        if (bytes.len() + n) as u64 > cap {
            return Err(ReadError::CapExceeded { high_water: bytes.len() as u64 });
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let high_water = bytes.len() as u64;
    Ok(CappedRead { bytes, high_water })
}

/// The archive file name minus its suffix; a name with no stem is refused.
fn output_name(file_name: &str, suffix_len: usize) -> Result<&str> {
    let stem = &file_name[..file_name.len() - suffix_len];
    if matches!(stem, "" | "." | "..") {
        return Err(ToolError::invalid_argument(format!(
            "cannot derive an output file name from '{file_name}'"
        )));
    }
    Ok(stem)
}

/// `normalize_path` clamps a leading `..` at the root (`../../x` becomes
/// `/x`), but a relative destination that climbs out of the project must be
/// refused (FR-NEW-011 item 6), so check the raw form first.
pub(crate) fn normalize_destination(state: &AppState, dest: &str) -> Result<String> {
    let climbs = !dest.starts_with('/')
        && crate::util::PosixPath::normpath(dest).split('/').next() == Some("..");
    if climbs {
        return Err(ToolError::path_out_of_bounds(format!("path escapes the volume root: {dest}")));
    }
    state.safety.normalize_path(dest)
}

/// Refuse a destination that is a file or sits under one, naming the first
/// non folder node from the root (`exists` cannot tell a file from a folder).
async fn check_destination(client: &VolumeClient, dest: &str) -> Result<()> {
    let mut path = String::new();
    for part in dest.split('/').filter(|p| !p.is_empty()) {
        path.push('/');
        path.push_str(part);
        match client.meta.get(&path).await? {
            Some(node) if !node.is_dir() => {
                return Err(ToolError::invalid_argument(format!(
                    "'{path}' is a file, not a folder"
                )));
            }
            Some(_) => {}
            // Nothing below a missing folder can exist.
            None => break,
        }
    }
    Ok(())
}

/// A folder at the output path is never replaced; a file needs `overwrite`.
async fn check_collision(client: &VolumeClient, out: &str, overwrite: bool) -> Result<()> {
    match client.meta.get(out).await? {
        Some(node) if node.is_dir() => {
            Err(ToolError::no_clobber(format!("'{out}' is a folder and cannot be replaced")))
        }
        Some(_) if !overwrite => Err(ToolError::no_clobber(format!(
            "'{out}' already exists; pass overwrite=true to replace it"
        ))),
        _ => Ok(()),
    }
}

/// Which limit a cap comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapKind {
    Project,
    Session,
}

/// The most output bytes the extraction may produce, and the limit that sets it.
#[derive(Debug, Clone, Copy)]
struct Caps {
    limit: u64,
    which: CapKind,
}

impl Caps {
    /// The refusal for the limit that was met first.
    async fn refusal(self, state: &AppState, client: &VolumeClient) -> Result<ToolError> {
        match self.which {
            CapKind::Project => {
                let limit = state.safety.project_limit(&client.project_id).await?.unwrap_or(0);
                let used = quota::used_bytes(client).await?;
                let requested = i64::try_from(self.limit).unwrap_or(i64::MAX).saturating_add(1);
                Ok(ToolError::project_quota_exceeded(limit, used, requested))
            }
            CapKind::Session => {
                let quota = state.safety.config().write_quota_bytes;
                Ok(ToolError::write_quota_exceeded(format!(
                    "session write quota of {quota} bytes exceeded"
                )))
            }
        }
    }
}

/// Output cap, computed before anything is read: the lower of the project's
/// remaining storage (plus the file an overwrite replaces, because the
/// admission counts net growth) and the session's remaining allowance (charged
/// gross). A tie goes to the project, which the admission also checks first.
async fn caps(
    state: &AppState,
    client: &VolumeClient,
    mount_id: &str,
    person: &str,
    out: &str,
    overwrite: bool,
) -> Result<Caps> {
    let replaced = match client.meta.get(out).await? {
        Some(node) if overwrite && node.is_file() => node.size,
        _ => 0,
    };
    let session = state.safety.session_remaining(person, mount_id).max(0) as u64;
    let project = quota::project_remaining_for(client, &state.safety, replaced).await?;
    Ok(match project.map(|p| p.max(0) as u64) {
        Some(p) if p <= session => Caps { limit: p, which: CapKind::Project },
        _ => Caps { limit: session, which: CapKind::Session },
    })
}

/// Extract the one file held by `normalized` next to it, or into
/// `destination`. Order: output name, destination, collision, then the read
/// and decode, admission and the writes; nothing is created and the archive
/// is not read before the checks passed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn extract_single(
    state: &AppState,
    client: &VolumeClient,
    mount_id: &str,
    person: &str,
    normalized: &str,
    destination: Option<&str>,
    overwrite: bool,
    format: SingleFormat,
    suffix_len: usize,
) -> Result<Value> {
    let (parent, file_name) = normalized.rsplit_once('/').unwrap_or(("", normalized));
    let name = output_name(file_name, suffix_len)?.to_string();

    let dest = match destination {
        Some(d) => normalize_destination(state, d)?,
        None if parent.is_empty() => "/".to_string(),
        None => parent.to_string(),
    };
    check_destination(client, &dest).await?;
    let out = crate::util::PosixPath::normpath(&format!("{}/{name}", dest.trim_end_matches('/')));
    check_collision(client, &out, overwrite).await?;

    let caps = caps(state, client, mount_id, person, &out, overwrite).await?;
    let bytes = client.read_bytes(normalized).await?;
    let label = format.label();
    let cap = caps.limit;
    let read = tokio::task::spawn_blocking(move || {
        decoder(format, &bytes).map_err(ReadError::Init).and_then(|r| read_capped(r, cap))
    })
    .await
    .map_err(|e| ToolError::internal(format!("archive decode task panicked: {e}")))?;
    let decoded = match read {
        Ok(decoded) => decoded,
        Err(ReadError::Io(e)) => return Err(corrupt(label, e)),
        Err(ReadError::Init(e)) => {
            return Err(ToolError::internal(format!("cannot start the {label} decoder: {e}")));
        }
        Err(ReadError::CapExceeded { high_water }) => {
            #[cfg(test)]
            LAST_HIGH_WATER.with(|c| c.set(Some(high_water)));
            tracing::debug!(path = %normalized, high_water, "single file decode stopped at the cap");
            return Err(caps.refusal(state, client).await?);
        }
    };
    #[cfg(test)]
    LAST_HIGH_WATER.with(|c| c.set(Some(decoded.high_water)));
    tracing::debug!(path = %normalized, high_water = decoded.high_water, "single file decoded");

    let len = decoded.bytes.len() as i64;
    let _admission =
        quota::admit_paths(client, &state.safety, person, mount_id, len, &[(out.clone(), len)])
            .await?;

    let dirs_created = u64::from(!client.exists(&dest).await?);
    client.makedirs(&dest, true).await?;
    crate::core::fs_ops::ensure_parents(client, &out).await?;
    client.write_bytes_atomic(&out, &decoded.bytes).await?;
    client.touch_atime_mtime(&out);

    finish_extraction(
        state,
        person,
        mount_id,
        normalized,
        &dest,
        1,
        decoded.bytes.len() as u64,
        dirs_created,
    )
}

#[cfg(test)]
mod tests {
    use crate::errors::{Result, code};
    use crate::tools::admin::test_support::Fixture;
    use crate::tools::archive::extract_archive;
    use serde_json::{Value, json};
    use std::io::Write as _;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "proj";
    const CSV: &[u8] = b"id,name\n1,Alice\n";

    fn gzip(content: &[u8]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(content).unwrap();
        gz.finish().unwrap()
    }

    async fn fixture() -> Fixture {
        let f = Fixture::with_config(|_| {}).await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        f
    }

    async fn seed(f: &Fixture, path: &str, content: &[u8]) {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic(path, content).await.unwrap();
    }

    async fn extract(f: &Fixture, path: &str, destination: Option<&str>) -> Result<Value> {
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        extract_archive(&f.state, MOUNT, OWNER, path, destination, false, None).await
    }

    async fn read(f: &Fixture, path: &str) -> Option<Vec<u8>> {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        if c.exists(path).await.unwrap() { Some(c.read_bytes(path).await.unwrap()) } else { None }
    }

    #[tokio::test]
    async fn e2e_0081_001_gzip_file_decompresses_next_to_the_a() {
        let f = fixture().await;
        let archive = gzip(CSV);
        seed(&f, "/uploads/data.csv.gz", &archive).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/uploads", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/data.csv.gz").await, Some(archive));
    }

    #[tokio::test]
    async fn e2e_0081_002_upper_case_suffix() {
        let f = fixture().await;
        seed(&f, "/uploads/DATA.CSV.GZ", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/DATA.CSV.GZ", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/DATA.CSV").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0081_003_explicit_existing_destination() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/out", true).await.unwrap();
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.gz", Some("/out")).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/out", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/out/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_004_missing_destination_is_created() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.gz", Some("/new/dir")).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/new/dir", "files_written": 1,
                         "dirs_created": 1, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/new/dir/data.csv").await.as_deref(), Some(CSV));
    }

    async fn refuses_name(file: &str, shown: &str) {
        let f = fixture().await;
        let path = format!("/uploads/{file}");
        seed(&f, &path, &gzip(CSV)).await;
        let e = extract(&f, &path, None).await;
        let e = e.expect_err("a name with no stem must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, format!("cannot derive an output file name from '{shown}'"));
        assert!(f.state.safety.audit(OWNER, MOUNT).iter().all(|a| a.op != "extract_archive"));
        // Nothing was created next to the archive: the folder still holds the archive alone.
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let names: Vec<String> =
            c.list_dir("/uploads").await.unwrap().into_iter().map(|n| n.path).collect();
        assert_eq!(names, vec![path.clone()], "{names:?}");
    }

    #[tokio::test]
    async fn e2e_0081_012_file_named_gz() {
        refuses_name(".gz", ".gz").await;
    }

    #[tokio::test]
    async fn e2e_0081_013_file_named_gz() {
        refuses_name("..gz", "..gz").await;
    }

    #[tokio::test]
    async fn e2e_0081_041_upper_case_gz_alone() {
        refuses_name(".GZ", ".GZ").await;
    }

    #[tokio::test]
    async fn e2e_0081_023_gzip_of_an_empty_file() {
        let f = fixture().await;
        seed(&f, "/uploads/empty.txt.gz", &gzip(b"")).await;
        let v = extract(&f, "/uploads/empty.txt.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        let v = v.unwrap_or_default();
        assert_eq!(v["files_written"], 1);
        assert_eq!(v["bytes_written"], 0);
        assert_eq!(read(&f, "/uploads/empty.txt").await, Some(Vec::new()));
    }

    #[tokio::test]
    async fn e2e_0081_026_accents_and_spaces() {
        let f = fixture().await;
        seed(&f, "/uploads/rapport été.txt.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/rapport été.txt.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/rapport été.txt").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0081_027_several_dots_and_no_dot() {
        let f = fixture().await;
        seed(&f, "/uploads/a.b.c.gz", &gzip(CSV)).await;
        seed(&f, "/uploads/README.gz", &gzip(CSV)).await;
        for p in ["/uploads/a.b.c.gz", "/uploads/README.gz"] {
            let v = extract(&f, p, None).await;
            assert!(v.is_ok(), "{p}: {v:?}");
        }
        assert_eq!(read(&f, "/uploads/a.b.c").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/README").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0081_030_success_is_audited() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        let log = f.state.safety.audit(OWNER, MOUNT);
        let lines: Vec<_> = log.iter().filter(|e| e.op == "extract_archive").collect();
        assert_eq!(lines.len(), 1, "{log:?}");
        assert_eq!(lines[0].path, "/uploads/data.csv.gz");
        assert_eq!(lines[0].detail, "destination=/uploads files_written=1 bytes_written=16");
    }

    /// Members written back to back, the shape `cat a.gz b.gz` produces.
    fn members(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| gzip(p)).collect()
    }

    /// A gzip file whose header carries a name, a comment and an extra field.
    fn gzip_with_header(content: &[u8], name: &str, comment: &str, extra: &[u8]) -> Vec<u8> {
        let mut gz = flate2::GzBuilder::new()
            .filename(name)
            .comment(comment)
            .extra(extra.to_vec())
            .write(Vec::new(), flate2::Compression::default());
        gz.write_all(content).unwrap();
        gz.finish().unwrap()
    }

    const CORRUPT: &str = "archive is corrupt or not a valid gz file:";

    fn assert_corrupt(v: &Result<Value>) {
        let e = v.as_ref().expect_err("a corrupt file must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
        assert!(e.message.starts_with(CORRUPT), "{}", e.message);
    }

    #[tokio::test]
    async fn e2e_0081_005_two_gzip_members_back_to_back() {
        let f = fixture().await;
        seed(&f, "/uploads/two.txt.gz", &members(&[b"hello ", b"world"])).await;
        let v = extract(&f, "/uploads/two.txt.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["bytes_written"], 11);
        assert_eq!(read(&f, "/uploads/two.txt").await.as_deref(), Some(&b"hello world"[..]));
    }

    #[tokio::test]
    async fn e2e_0081_008_truncated_file() {
        let f = fixture().await;
        let whole = gzip(CSV);
        seed(&f, "/uploads/cut.gz", &whole[..whole.len() / 2]).await;
        let v = extract(&f, "/uploads/cut.gz", None).await;
        assert_corrupt(&v);
        assert_eq!(read(&f, "/uploads/cut").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_009_text_file_named_gz() {
        let f = fixture().await;
        seed(&f, "/uploads/fake.gz", b"not gzip").await;
        let v = extract(&f, "/uploads/fake.gz", None).await;
        assert_corrupt(&v);
        assert_eq!(read(&f, "/uploads/fake").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_010_bytes_after_a_valid_stream() {
        let f = fixture().await;
        let mut junk = gzip(CSV);
        junk.extend_from_slice(b"JUNK");
        seed(&f, "/uploads/tail.gz", &junk).await;
        let v = extract(&f, "/uploads/tail.gz", None).await;
        assert_corrupt(&v);
        assert_eq!(read(&f, "/uploads/tail").await, None);

        let mut zeros = gzip(CSV);
        zeros.extend_from_slice(&[0u8; 4]);
        seed(&f, "/uploads/pad.gz", &zeros).await;
        let v = extract(&f, "/uploads/pad.gz", None).await;
        assert_corrupt(&v);
        assert_eq!(read(&f, "/uploads/pad").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_018_name_stored_inside_the_file_is_ignor() {
        let f = fixture().await;
        let archive = gzip_with_header(CSV, "../../etc/passwd", "", b"");
        seed(&f, "/uploads/data.csv.gz", &archive).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/etc/passwd").await, None);
        assert_eq!(read(&f, "/uploads/etc").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_033_a_refused_extraction_creates_nothing() {
        let f = fixture().await;
        let whole = gzip(CSV);
        seed(&f, "/uploads/cut.gz", &whole[..whole.len() / 2]).await;
        let v = extract(&f, "/uploads/cut.gz", Some("/fresh")).await;
        assert_corrupt(&v);
        let c = f.state.stores.client(MOUNT).await.unwrap();
        assert!(!c.exists("/fresh").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_0081_037_three_gzip_members() {
        let f = fixture().await;
        seed(&f, "/uploads/three.txt.gz", &members(&[b"a", b"b", b"c"])).await;
        let v = extract(&f, "/uploads/three.txt.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["bytes_written"], 3);
        assert_eq!(read(&f, "/uploads/three.txt").await.as_deref(), Some(&b"abc"[..]));
    }

    #[tokio::test]
    async fn e2e_0081_038_an_empty_gzip_member_between_two_oth() {
        let f = fixture().await;
        seed(&f, "/uploads/gap.txt.gz", &members(&[b"a", b"", b"b"])).await;
        let v = extract(&f, "/uploads/gap.txt.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/gap.txt").await.as_deref(), Some(&b"ab"[..]));
    }

    #[tokio::test]
    async fn e2e_0081_042_a_different_stored_name() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip_with_header(CSV, "other.txt", "", b"")).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/other.txt").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_043_comment_and_extra_fields_in_the_head() {
        let f = fixture().await;
        let archive = gzip_with_header(CSV, "x.txt", "from alice", b"\x01\x02\x03");
        seed(&f, "/uploads/data.csv.gz", &archive).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let mut names: Vec<String> =
            c.list_dir("/uploads").await.unwrap().into_iter().map(|n| n.path).collect();
        names.sort();
        assert_eq!(names, vec!["/uploads/data.csv", "/uploads/data.csv.gz"], "{names:?}");
    }

    #[tokio::test]
    async fn e2e_0081_046_empty_file() {
        let f = fixture().await;
        seed(&f, "/uploads/zero.gz", b"").await;
        let v = extract(&f, "/uploads/zero.gz", None).await;
        assert_corrupt(&v);
        assert_eq!(read(&f, "/uploads/zero").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_036_trailing_slash_on_the_destination() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/out", true).await.unwrap();
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.gz", Some("/out/")).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["destination"], "/out");
        assert_eq!(read(&f, "/out/data.csv").await.as_deref(), Some(CSV));
    }

    async fn extract_with(
        f: &Fixture,
        path: &str,
        destination: Option<&str>,
        overwrite: bool,
        password: Option<&str>,
    ) -> Result<Value> {
        f.state.authorize(MOUNT, OWNER).await.unwrap();
        extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password).await
    }

    async fn exists(f: &Fixture, path: &str) -> bool {
        f.state.stores.client(MOUNT).await.unwrap().exists(path).await.unwrap()
    }

    const NO_PASSWORD: &str = "password is not applicable to this archive format";

    #[tokio::test]
    async fn e2e_0081_006_collision() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        seed(&f, "/uploads/data.csv", b"old").await;
        let e = extract_with(&f, "/uploads/data.csv.gz", None, false, None).await;
        let e = e.expect_err("an existing output must be refused");
        assert_eq!(e.code, code::NO_CLOBBER);
        assert_eq!(
            e.message,
            "'/uploads/data.csv' already exists; pass overwrite=true to replace it"
        );
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(&b"old"[..]));
    }

    #[tokio::test]
    async fn e2e_0081_011_password_given() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let e = extract_with(&f, "/uploads/data.csv.gz", None, false, Some("s3cret")).await;
        let e = e.expect_err("a password must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, NO_PASSWORD);
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_039_empty_password() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let e = extract_with(&f, "/uploads/data.csv.gz", None, false, Some("")).await;
        let e = e.expect_err("an empty password must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, NO_PASSWORD);
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    #[tokio::test]
    async fn e2e_0081_024_overwrite_replaces() {
        let f = fixture().await;
        let archive = gzip(CSV);
        seed(&f, "/uploads/data.csv.gz", &archive).await;
        seed(&f, "/uploads/data.csv", &[b'x'; 40]).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let used = crate::quota::used_bytes(&c).await.unwrap();
        f.state.admin.set_quota(MOUNT, Some(used + 5)).await.unwrap();
        let v = extract_with(&f, "/uploads/data.csv.gz", None, true, None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(crate::quota::used_bytes(&c).await.unwrap(), used - 24);
    }

    #[tokio::test]
    async fn e2e_0081_034_a_folder_bears_the_output_name() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/data.csv", true).await.unwrap();
        for overwrite in [true, false] {
            let e = extract_with(&f, "/uploads/data.csv.gz", None, overwrite, None).await;
            let e = e.expect_err("a folder must never be replaced");
            assert_eq!(e.code, code::NO_CLOBBER);
            assert_eq!(e.message, "'/uploads/data.csv' is a folder and cannot be replaced");
        }
        let node = c.meta.get("/uploads/data.csv").await.unwrap();
        assert!(node.is_some_and(|n| n.is_dir()));
    }

    async fn tar_gz_is_a_tar(archive: &str, folder: &str, files: u64) {
        let f = fixture().await;
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut h = tar::Header::new_gnu();
            h.set_path("a.txt").unwrap();
            h.set_size(5);
            h.set_cksum();
            b.append(&h, &b"hello"[..]).unwrap();
            b.finish().unwrap();
        }
        seed(&f, archive, &gzip(&tar_bytes)).await;
        let v = extract_with(&f, archive, None, false, None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": folder, "files_written": files,
                         "dirs_created": 1, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, &format!("{folder}/a.txt")).await.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn e2e_0081_028_tar_gz_is_still_a_tar() {
        tar_gz_is_a_tar("/uploads/pack.tar.gz", "/uploads/pack", 1).await;
    }

    #[tokio::test]
    async fn e2e_0081_029_tgz_is_still_a_tar() {
        tar_gz_is_a_tar("/uploads/pack.tgz", "/uploads/pack", 1).await;
    }

    #[tokio::test]
    async fn e2e_0081_044_tar_gz_is_still_a_tar() {
        tar_gz_is_a_tar("/uploads/PACK.TAR.GZ", "/uploads/PACK", 1).await;
    }

    async fn destination_refused(destination: &str) {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        seed(&f, "/uploads/notes.txt", b"keep").await;
        let e = extract_with(&f, "/uploads/data.csv.gz", Some(destination), false, None).await;
        let e = e.expect_err("a file destination must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "'/uploads/notes.txt' is a file, not a folder");
        assert_eq!(read(&f, "/uploads/notes.txt").await.as_deref(), Some(&b"keep"[..]));
        assert!(!exists(&f, "/uploads/notes.txt/sub").await);
    }

    #[tokio::test]
    async fn e2e_0081_054_destination_is_a_file() {
        destination_refused("/uploads/notes.txt").await;
    }

    #[tokio::test]
    async fn e2e_0081_055_a_file_among_the_ancestors_of_the_de() {
        destination_refused("/uploads/notes.txt/sub").await;
    }

    // Quota and session cap tests (US-0163).

    const BOMB_LEN: usize = 10_000_000;

    fn bomb() -> Vec<u8> {
        gzip(&vec![0u8; BOMB_LEN])
    }

    /// Fixture whose session write allowance is `n` bytes.
    async fn fixture_allowance(n: i64) -> Fixture {
        let f = Fixture::with_config(move |c| c.safety.write_quota_bytes = n).await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        f
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

    async fn write_small(f: &Fixture) {
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let v = crate::core::fs_ops::write_bytes(
            &c,
            &f.state.safety,
            OWNER,
            MOUNT,
            "/uploads/small.txt",
            b"abc",
            false,
            true,
        )
        .await;
        assert!(v.is_ok(), "{v:?}");
    }

    fn project_refusal(v: &Result<Value>) {
        let e = v.as_ref().expect_err("the extraction must be refused");
        assert_eq!(e.code, code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
        assert!(e.message.starts_with("project storage quota of"), "{}", e.message);
    }

    fn session_refusal(v: &Result<Value>, quota: i64) {
        let e = v.as_ref().expect_err("the extraction must be refused");
        assert_eq!(e.code, code::WRITE_QUOTA_EXCEEDED, "{e:?}");
        assert_eq!(e.message, format!("session write quota of {quota} bytes exceeded"));
    }

    /// The bomb is stopped by the decoder itself: it never holds more than
    /// the cap plus one chunk.
    fn assert_bomb_bounded(cap: u64) {
        let err =
            super::read_capped(super::decoder(super::SingleFormat::Gzip, &bomb()).unwrap(), cap)
                .expect_err("the bomb is over the cap");
        match err {
            super::ReadError::CapExceeded { high_water } => {
                assert!(high_water <= cap + 65_536, "held {high_water} bytes");
                assert!(high_water < 1_000_000, "held {high_water} bytes");
            }
            super::ReadError::Io(e) | super::ReadError::Init(e) => {
                panic!("expected the cap error, got {e}")
            }
        }
    }

    /// The decode inside the extraction call just made held at most the cap
    /// plus one chunk (the tokio test runtime is single threaded, so the
    /// thread local belongs to this test alone).
    fn assert_extraction_bounded(cap: u64) {
        let held = super::LAST_HIGH_WATER.with(std::cell::Cell::take);
        let held = held.expect("the extraction decoded nothing");
        assert!(held <= cap + 65_536, "the extraction held {held} bytes");
    }

    #[tokio::test]
    async fn e2e_0081_007_output_over_the_remaining_quota() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        limit_remaining(&f, 10).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        project_refusal(&v);
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
        assert_eq!(remaining_storage(&f).await, 10);
    }

    #[tokio::test]
    async fn e2e_0081_017_decompression_bomb() {
        let f = fixture().await;
        seed(&f, "/uploads/bomb.gz", &bomb()).await;
        limit_remaining(&f, 1000).await;
        let v = extract(&f, "/uploads/bomb.gz", None).await;
        project_refusal(&v);
        assert_eq!(read(&f, "/uploads/bomb").await, None);
        assert_eq!(remaining_storage(&f).await, 1000);
        assert_bomb_bounded(1000);
        assert_extraction_bounded(1000);
    }

    #[tokio::test]
    async fn e2e_0081_025_output_exactly_equal_to_the_remainin() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        limit_remaining(&f, 16).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(remaining_storage(&f).await, 0);
    }

    #[tokio::test]
    async fn e2e_0081_032_a_refused_extraction_leaves_the_proj() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        limit_remaining(&f, 10).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        project_refusal(&v);
        write_small(&f).await;
        assert_eq!(remaining_storage(&f).await, 7);
    }

    #[tokio::test]
    async fn e2e_0081_047_one_byte_over_the_remaining_storage() {
        let f = fixture().await;
        seed(&f, "/uploads/odd.gz", &gzip(b"id,name\n1,Alice\nx")).await;
        limit_remaining(&f, 16).await;
        let v = extract(&f, "/uploads/odd.gz", None).await;
        project_refusal(&v);
        assert_eq!(read(&f, "/uploads/odd").await, None);
        assert_eq!(remaining_storage(&f).await, 16);
    }

    #[tokio::test]
    async fn e2e_0081_048_bomb_against_the_session_allowance() {
        let f = fixture_allowance(1000).await;
        seed(&f, "/uploads/bomb.gz", &bomb()).await;
        let v = extract(&f, "/uploads/bomb.gz", None).await;
        session_refusal(&v, 1000);
        assert_eq!(read(&f, "/uploads/bomb").await, None);
        assert_eq!(allowance(&f), 1000);
        assert_bomb_bounded(1000);
        assert_extraction_bounded(1000);
    }

    #[tokio::test]
    async fn e2e_0081_049_the_allowance_is_charged_the_full_ou() {
        let f = fixture_allowance(20).await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        seed(&f, "/uploads/data.csv", &[b'x'; 40]).await;
        let v = extract_with(&f, "/uploads/data.csv.gz", None, true, None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(allowance(&f), 4);
    }

    #[tokio::test]
    async fn e2e_0081_050_a_refused_extraction_charges_nothing() {
        let f = fixture_allowance(10).await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        session_refusal(&v, 10);
        write_small(&f).await;
        assert_eq!(allowance(&f), 7);
    }

    #[tokio::test]
    async fn e2e_0081_051_both_limits_at_the_same_byte() {
        let f = fixture_allowance(10).await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        limit_remaining(&f, 10).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        project_refusal(&v);
        assert_eq!(allowance(&f), 10);
    }

    #[tokio::test]
    async fn e2e_0081_052_the_lower_limit_is_met_first() {
        let f = fixture_allowance(100).await;
        seed(&f, "/uploads/bomb.gz", &bomb()).await;
        limit_remaining(&f, 1000).await;
        let v = extract(&f, "/uploads/bomb.gz", None).await;
        session_refusal(&v, 100);
        assert_eq!(remaining_storage(&f).await, 1000);
        assert_eq!(allowance(&f), 100);
    }

    #[tokio::test]
    async fn e2e_0081_053_allowance_equal_to_the_output() {
        let f = fixture_allowance(16).await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.gz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(allowance(&f), 0);
    }

    // Order of refusals (US-0164, FR-NEW-011).

    const BOB: &str = "bob@example.com";

    /// `alpha` has its own owner; `bob@example.com` is not a member. The door
    /// authorizes before the engine, so the helper does too.
    async fn alpha_fixture() -> Fixture {
        let f = fixture().await;
        f.seed_project("alpha", OWNER).await;
        let c = f.state.stores.client("alpha").await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        c.write_bytes_atomic("/uploads/data.csv.gz", &gzip(CSV)).await.unwrap();
        f
    }

    async fn extract_as_bob(f: &Fixture, path: &str) -> Result<Value> {
        f.state.authorize("alpha", BOB).await?;
        extract_archive(&f.state, "alpha", BOB, path, None, false, None).await
    }

    fn refused(v: &Result<Value>, code: &str, message: &str) {
        let e = v.as_ref().expect_err("the extraction must be refused");
        assert_eq!(e.code, code, "{e:?}");
        assert_eq!(e.message, message);
    }

    /// A gzip stream whose output is 1000 bytes, cut before its trailer.
    fn truncated_gzip() -> Vec<u8> {
        let mut bytes = gzip(&[0u8; 1000]);
        bytes.truncate(bytes.len() - 8);
        bytes
    }

    #[tokio::test]
    async fn e2e_0081_014_missing_archive() {
        let f = fixture().await;
        let v = extract(&f, "/uploads/ghost.gz", None).await;
        refused(&v, code::NOT_FOUND, "'/uploads/ghost.gz' not found");
    }

    #[tokio::test]
    async fn e2e_0081_015_folder_named_like_an_archive() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/dir.gz", true).await.unwrap();
        let v = extract(&f, "/uploads/dir.gz", None).await;
        refused(
            &v,
            code::INVALID_ARGUMENT,
            "'/uploads/dir.gz' is a directory, not an archive file",
        );
    }

    #[tokio::test]
    async fn e2e_0081_016_non_member() {
        let f = alpha_fixture().await;
        let v = extract_as_bob(&f, "/uploads/data.csv.gz").await;
        refused(&v, code::FORBIDDEN, "'bob@example.com' is not a member of 'alpha'");
        let c = f.state.stores.client("alpha").await.unwrap();
        assert!(!c.exists("/uploads/data.csv").await.unwrap());
    }

    #[tokio::test]
    async fn e2e_0081_019_membership_wins_over_a_missing_path() {
        let f = alpha_fixture().await;
        let v = extract_as_bob(&f, "/uploads/ghost.gz").await;
        refused(&v, code::FORBIDDEN, "'bob@example.com' is not a member of 'alpha'");
    }

    #[tokio::test]
    async fn e2e_0081_020_password_wins_over_a_corrupt_file() {
        let f = fixture().await;
        seed(&f, "/uploads/fake.gz", b"not gzip").await;
        let v = extract_with(&f, "/uploads/fake.gz", None, false, Some("s3cret")).await;
        refused(&v, code::INVALID_ARGUMENT, NO_PASSWORD);
    }

    #[tokio::test]
    async fn e2e_0081_021_collision_wins_over_a_corrupt_file() {
        let f = fixture().await;
        seed(&f, "/uploads/cut.gz", &truncated_gzip()).await;
        seed(&f, "/uploads/cut", b"old").await;
        let v = extract_with(&f, "/uploads/cut.gz", None, false, None).await;
        refused(
            &v,
            code::NO_CLOBBER,
            "'/uploads/cut' already exists; pass overwrite=true to replace it",
        );
    }

    #[tokio::test]
    async fn e2e_0081_022_collision_wins_over_the_storage_limi() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.gz", &gzip(CSV)).await;
        seed(&f, "/uploads/data.csv", b"old").await;
        limit_remaining(&f, 10).await;
        let v = extract_with(&f, "/uploads/data.csv.gz", None, false, None).await;
        refused(
            &v,
            code::NO_CLOBBER,
            "'/uploads/data.csv' already exists; pass overwrite=true to replace it",
        );
    }

    #[tokio::test]
    async fn e2e_0081_035_destination_outside_the_project() {
        let f = fixture().await;
        seed(&f, "/uploads/cut.gz", &truncated_gzip()).await;
        let v = extract_with(&f, "/uploads/cut.gz", Some("../../x"), false, None).await;
        refused(&v, code::PATH_OUT_OF_BOUNDS, "path escapes the volume root: ../../x");
    }

    #[tokio::test]
    async fn e2e_0081_045_storage_limit_met_before_the_corrupt() {
        let f = fixture().await;
        seed(&f, "/uploads/big.gz", &truncated_gzip()).await;
        limit_remaining(&f, 10).await;
        let v = extract(&f, "/uploads/big.gz", None).await;
        project_refusal(&v);
        assert_eq!(read(&f, "/uploads/big").await, None);
    }

    // Bare bzip2 files (SPEC-0083, US-0165).

    fn bzip2(content: &[u8]) -> Vec<u8> {
        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        bz.write_all(content).unwrap();
        bz.finish().unwrap()
    }

    #[tokio::test]
    async fn e2e_0083_001_bzip2_file_decompresses_next_to_the() {
        let f = fixture().await;
        let archive = bzip2(CSV);
        seed(&f, "/uploads/data.csv.bz2", &archive).await;
        let v = extract(&f, "/uploads/data.csv.bz2", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/uploads", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/data.csv.bz2").await, Some(archive));
    }

    #[tokio::test]
    async fn e2e_0083_002_upper_case_suffix() {
        let f = fixture().await;
        seed(&f, "/uploads/DATA.CSV.BZ2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/DATA.CSV.BZ2", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/DATA.CSV").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0083_003_explicit_existing_destination() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/out", true).await.unwrap();
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.bz2", Some("/out")).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/out", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/out/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    async fn refuses_bz2_name(file: &str, shown: &str) {
        let f = fixture().await;
        let path = format!("/uploads/{file}");
        seed(&f, &path, &bzip2(CSV)).await;
        let e = extract(&f, &path, None).await;
        let e = e.expect_err("a name with no stem must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, format!("cannot derive an output file name from '{shown}'"));
        assert!(f.state.safety.audit(OWNER, MOUNT).iter().all(|a| a.op != "extract_archive"));
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let names: Vec<String> =
            c.list_dir("/uploads").await.unwrap().into_iter().map(|n| n.path).collect();
        assert_eq!(names, vec![path.clone()], "{names:?}");
    }

    #[tokio::test]
    async fn e2e_0083_023_file_named_bz2() {
        refuses_bz2_name(".bz2", ".bz2").await;
    }

    #[tokio::test]
    async fn e2e_0083_024_file_named_bz2() {
        refuses_bz2_name("..bz2", "..bz2").await;
    }

    #[tokio::test]
    async fn e2e_0083_025_upper_case_bz2_alone() {
        refuses_bz2_name(".BZ2", ".BZ2").await;
    }

    /// Green on arrival: `.lz` was already refused before bzip2 existed.
    #[tokio::test]
    async fn e2e_0083_038_another_extension_is_still_refused() {
        let f = fixture().await;
        seed(&f, "/uploads/pack.lz", b"anything").await;
        let e = extract(&f, "/uploads/pack.lz", None).await;
        let e = e.expect_err("an unknown extension must be refused");
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert_eq!(e.message, "unsupported archive extension: '.lz'");
    }

    #[tokio::test]
    async fn e2e_0083_039_bzip2_of_an_empty_file() {
        let f = fixture().await;
        seed(&f, "/uploads/empty.txt.bz2", &bzip2(b"")).await;
        let v = extract(&f, "/uploads/empty.txt.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        let v = v.unwrap_or_default();
        assert_eq!(v["files_written"], 1);
        assert_eq!(v["bytes_written"], 0);
        assert_eq!(read(&f, "/uploads/empty.txt").await, Some(Vec::new()));
    }

    #[tokio::test]
    async fn e2e_0083_043_accents_and_spaces() {
        let f = fixture().await;
        seed(&f, "/uploads/rapport été.txt.bz2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/rapport été.txt.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/rapport été.txt").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0083_044_several_dots_and_no_dot() {
        let f = fixture().await;
        seed(&f, "/uploads/a.b.c.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/README.bz2", &bzip2(CSV)).await;
        for p in ["/uploads/a.b.c.bz2", "/uploads/README.bz2"] {
            let v = extract(&f, p, None).await;
            assert!(v.is_ok(), "{p}: {v:?}");
        }
        assert_eq!(read(&f, "/uploads/a.b.c").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/README").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0083_045_archive_directly_under_the_project_r() {
        let f = fixture().await;
        seed(&f, "/a.bz2", &bzip2(b"hello")).await;
        let v = extract(&f, "/a.bz2", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/a").await.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_046_missing_destination_is_created() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.bz2", Some("/new/dir")).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/new/dir", "files_written": 1,
                         "dirs_created": 1, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/new/dir/data.csv").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0083_047_trailing_slash_on_the_destination() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/out", true).await.unwrap();
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.bz2", Some("/out/")).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["destination"], "/out");
        assert_eq!(read(&f, "/out/data.csv").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0083_051_success_is_audited() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        let log = f.state.safety.audit(OWNER, MOUNT);
        let lines: Vec<_> = log.iter().filter(|e| e.op == "extract_archive").collect();
        assert_eq!(lines.len(), 1, "{log:?}");
        assert_eq!(lines[0].path, "/uploads/data.csv.bz2");
        assert_eq!(lines[0].detail, "destination=/uploads files_written=1 bytes_written=16");
    }

    fn bz_streams(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| bzip2(p)).collect()
    }

    const BZ_CORRUPT: &str = "archive is corrupt or not a valid bz2 file:";

    fn assert_bz_corrupt(v: &Result<Value>) {
        let e = v.as_ref().expect_err("a corrupt file must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
        assert!(e.message.starts_with(BZ_CORRUPT), "{}", e.message);
    }

    async fn refused_bz(f: &Fixture, archive: &str, output: &str) {
        let v = extract(f, archive, None).await;
        assert_bz_corrupt(&v);
        assert_eq!(read(f, output).await, None);
    }

    #[tokio::test]
    async fn e2e_0083_004_two_bzip2_streams_back_to_back() {
        let f = fixture().await;
        seed(&f, "/uploads/two.txt.bz2", &bz_streams(&[b"hello ", b"world"])).await;
        let v = extract(&f, "/uploads/two.txt.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["bytes_written"], 11);
        assert_eq!(read(&f, "/uploads/two.txt").await.as_deref(), Some(&b"hello world"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_005_three_bzip2_streams() {
        let f = fixture().await;
        seed(&f, "/uploads/three.txt.bz2", &bz_streams(&[b"a", b"b", b"c"])).await;
        let v = extract(&f, "/uploads/three.txt.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["bytes_written"], 3);
        assert_eq!(read(&f, "/uploads/three.txt").await.as_deref(), Some(&b"abc"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_048_an_empty_bzip2_stream_between_two_ot() {
        let f = fixture().await;
        seed(&f, "/uploads/hole.txt.bz2", &bz_streams(&[b"a", b"", b"b"])).await;
        let v = extract(&f, "/uploads/hole.txt.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/hole.txt").await.as_deref(), Some(&b"ab"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_014_truncated_file() {
        let f = fixture().await;
        let whole = bzip2(CSV);
        seed(&f, "/uploads/cut.bz2", &whole[..whole.len() / 2]).await;
        refused_bz(&f, "/uploads/cut.bz2", "/uploads/cut").await;
    }

    #[tokio::test]
    async fn e2e_0083_015_text_file_named_bz2() {
        let f = fixture().await;
        seed(&f, "/uploads/fake.bz2", b"not bzip2").await;
        refused_bz(&f, "/uploads/fake.bz2", "/uploads/fake").await;
    }

    #[tokio::test]
    async fn e2e_0083_016_bytes_after_a_valid_stream() {
        let f = fixture().await;
        let mut junk = bzip2(CSV);
        junk.extend_from_slice(b"JUNK");
        seed(&f, "/uploads/tail.bz2", &junk).await;
        refused_bz(&f, "/uploads/tail.bz2", "/uploads/tail").await;
    }

    #[tokio::test]
    async fn e2e_0083_017_zero_bytes_after_a_valid_stream() {
        let f = fixture().await;
        let mut zeros = bzip2(CSV);
        zeros.extend_from_slice(&[0u8; 4]);
        seed(&f, "/uploads/pad.bz2", &zeros).await;
        refused_bz(&f, "/uploads/pad.bz2", "/uploads/pad").await;
    }

    #[tokio::test]
    async fn e2e_0083_018_damaged_checksum() {
        let f = fixture().await;
        let mut bad = bzip2(CSV);
        // Past the 4 byte stream header and 10 byte block header, inside the compressed data.
        let at = bad.len() / 2;
        bad[at] ^= 0xFF;
        seed(&f, "/uploads/bad.bz2", &bad).await;
        refused_bz(&f, "/uploads/bad.bz2", "/uploads/bad").await;
    }

    #[tokio::test]
    async fn e2e_0083_019_a_0_byte_file_named_zero_bz2() {
        let f = fixture().await;
        seed(&f, "/uploads/zero.bz2", b"").await;
        refused_bz(&f, "/uploads/zero.bz2", "/uploads/zero").await;
    }

    async fn password_refused(password: &str) {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let e = extract_with(&f, "/uploads/data.csv.bz2", None, false, Some(password)).await;
        let e = e.expect_err("a password must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, NO_PASSWORD);
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    #[tokio::test]
    async fn e2e_0083_020_password_given() {
        password_refused("s3cret").await;
    }

    #[tokio::test]
    async fn e2e_0083_021_empty_password() {
        password_refused("").await;
    }

    async fn tar_bz2_is_a_tar(archive: &str, folder: &str) {
        let f = fixture().await;
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut h = tar::Header::new_gnu();
            h.set_path("a.txt").unwrap();
            h.set_size(5);
            h.set_cksum();
            b.append(&h, &b"hello"[..]).unwrap();
            b.finish().unwrap();
        }
        seed(&f, archive, &bzip2(&tar_bytes)).await;
        let v = extract_with(&f, archive, None, false, None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": folder, "files_written": 1,
                         "dirs_created": 1, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, &format!("{folder}/a.txt")).await.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_006_tar_bz2_is_still_a_tar() {
        tar_bz2_is_a_tar("/uploads/pack.tar.bz2", "/uploads/pack").await;
    }

    #[tokio::test]
    async fn e2e_0083_007_tb2_is_still_a_tar() {
        tar_bz2_is_a_tar("/uploads/pack.tb2", "/uploads/pack").await;
    }

    #[tokio::test]
    async fn e2e_0083_008_tar_bz2_is_still_a_tar() {
        tar_bz2_is_a_tar("/uploads/PACK.TAR.BZ2", "/uploads/PACK").await;
    }

    // Collision, destination and order of refusals for bzip2 (US-0167).

    const BZ_EXISTS: &str = "'/uploads/data.csv' already exists; pass overwrite=true to replace it";

    /// A bzip2 stream of `CSV` cut in half.
    fn truncated_bzip2() -> Vec<u8> {
        let whole = bzip2(CSV);
        whole[..whole.len() / 2].to_vec()
    }

    #[tokio::test]
    async fn e2e_0083_009_collision() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/data.csv", b"old").await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", None, false, None).await;
        refused(&v, code::NO_CLOBBER, BZ_EXISTS);
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(&b"old"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_010_a_folder_bears_the_output_name() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/data.csv", true).await.unwrap();
        let v = extract_with(&f, "/uploads/data.csv.bz2", None, true, None).await;
        refused(&v, code::NO_CLOBBER, "'/uploads/data.csv' is a folder and cannot be replaced");
        let node = c.meta.get("/uploads/data.csv").await.unwrap();
        assert!(node.is_some_and(|n| n.is_dir()));
    }

    #[tokio::test]
    async fn e2e_0083_026_missing_archive() {
        let f = fixture().await;
        let v = extract_with(&f, "/uploads/ghost.bz2", None, false, None).await;
        refused(&v, code::NOT_FOUND, "'/uploads/ghost.bz2' not found");
    }

    #[tokio::test]
    async fn e2e_0083_027_folder_named_like_an_archive() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/uploads/dir.bz2", true).await.unwrap();
        let v = extract_with(&f, "/uploads/dir.bz2", None, false, None).await;
        refused(
            &v,
            code::INVALID_ARGUMENT,
            "'/uploads/dir.bz2' is a directory, not an archive file",
        );
    }

    #[tokio::test]
    async fn e2e_0083_028_non_member() {
        let f = fixture().await;
        f.seed_project("alpha", OWNER).await;
        let c = f.state.stores.client("alpha").await.unwrap();
        c.makedirs("/uploads", true).await.unwrap();
        c.write_bytes_atomic("/uploads/data.csv.bz2", &bzip2(CSV)).await.unwrap();
        let v = extract_as_bob(&f, "/uploads/data.csv.bz2").await;
        refused(&v, code::FORBIDDEN, "'bob@example.com' is not a member of 'alpha'");
        assert!(!c.exists("/uploads/data.csv").await.unwrap());
    }

    async fn bz_destination_refused(destination: &str) {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/notes.txt", b"keep").await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", Some(destination), false, None).await;
        refused(&v, code::INVALID_ARGUMENT, "'/uploads/notes.txt' is a file, not a folder");
        assert_eq!(read(&f, "/uploads/notes.txt").await.as_deref(), Some(&b"keep"[..]));
        assert!(!exists(&f, "/uploads/notes.txt/sub").await);
        assert!(!exists(&f, "/uploads/data.csv").await);
    }

    #[tokio::test]
    async fn e2e_0083_029_destination_is_a_file() {
        bz_destination_refused("/uploads/notes.txt").await;
    }

    #[tokio::test]
    async fn e2e_0083_030_a_file_among_the_ancestors_of_the_de() {
        bz_destination_refused("/uploads/notes.txt/sub").await;
    }

    #[tokio::test]
    async fn e2e_0083_032_destination_outside_the_project_wins() {
        let f = fixture().await;
        seed(&f, "/uploads/fake.bz2", b"not bzip2").await;
        let v = extract_with(&f, "/uploads/fake.bz2", Some("../../x"), false, None).await;
        refused(&v, code::PATH_OUT_OF_BOUNDS, "path escapes the volume root: ../../x");
        assert!(!exists(&f, "/x").await);
        assert!(!exists(&f, "/uploads/fake").await);
    }

    #[tokio::test]
    async fn e2e_0083_033_password_wins_over_a_corrupt_file() {
        let f = fixture().await;
        seed(&f, "/uploads/fake.bz2", b"not bzip2").await;
        let v = extract_with(&f, "/uploads/fake.bz2", None, false, Some("s3cret")).await;
        refused(&v, code::INVALID_ARGUMENT, NO_PASSWORD);
    }

    #[tokio::test]
    async fn e2e_0083_034_collision_wins_over_a_corrupt_file() {
        let f = fixture().await;
        seed(&f, "/uploads/cut.bz2", &truncated_bzip2()).await;
        seed(&f, "/uploads/cut", b"old").await;
        let v = extract_with(&f, "/uploads/cut.bz2", None, false, None).await;
        refused(
            &v,
            code::NO_CLOBBER,
            "'/uploads/cut' already exists; pass overwrite=true to replace it",
        );
        assert_eq!(read(&f, "/uploads/cut").await.as_deref(), Some(&b"old"[..]));
    }

    #[tokio::test]
    async fn e2e_0083_035_collision_wins_over_the_storage_limi() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/data.csv", b"old").await;
        limit_remaining(&f, 10).await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", None, false, None).await;
        refused(&v, code::NO_CLOBBER, BZ_EXISTS);
    }

    /// Pins the outcome the spec states (the file refusal, not `ERR_NO_CLOBBER`). It cannot pin
    /// the ORDER of the two checks: under a destination that is a file the output path
    /// `/uploads/notes.txt/data.csv` can never exist, so no collision can apply (BL-0128).
    #[tokio::test]
    async fn e2e_0083_036_destination_is_a_file_wins_over_a_co() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/notes.txt", b"keep").await;
        seed(&f, "/uploads/data.csv", b"old").await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", Some("/uploads/notes.txt"), false, None)
            .await;
        refused(&v, code::INVALID_ARGUMENT, "'/uploads/notes.txt' is a file, not a folder");
    }

    #[tokio::test]
    async fn e2e_0083_037_unusable_name_wins_over_a_corrupt_fi() {
        let f = fixture().await;
        seed(&f, "/uploads/.bz2", b"not bzip2").await;
        let v = extract_with(&f, "/uploads/.bz2", None, false, None).await;
        refused(&v, code::INVALID_ARGUMENT, "cannot derive an output file name from '.bz2'");
    }

    #[tokio::test]
    async fn e2e_0083_050_overwrite_replaces_a_file() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/data.csv", b"old").await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", None, true, None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(CSV.len(), 16);
    }

    // Storage and session limits on bare bzip2 files (SPEC-0083, US-0168).

    fn bz_bomb() -> Vec<u8> {
        bzip2(&vec![0u8; BOMB_LEN])
    }

    #[tokio::test]
    async fn e2e_0083_011_output_over_the_project_storage_limi() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        limit_remaining(&f, 10).await;
        let v = extract(&f, "/uploads/data.csv.bz2", None).await;
        project_refusal(&v);
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
        assert_eq!(remaining_storage(&f).await, 10);
    }

    #[tokio::test]
    async fn e2e_0083_012_decompression_bomb_against_the_stora() {
        let f = fixture().await;
        seed(&f, "/uploads/bomb.bz2", &bz_bomb()).await;
        limit_remaining(&f, 1000).await;
        let v = extract(&f, "/uploads/bomb.bz2", None).await;
        project_refusal(&v);
        assert_eq!(read(&f, "/uploads/bomb").await, None);
        assert_eq!(remaining_storage(&f).await, 1000);
        assert_extraction_bounded(1000);
    }

    #[tokio::test]
    async fn e2e_0083_013_decompression_bomb_against_the_sessi() {
        let f = fixture_allowance(1000).await;
        seed(&f, "/uploads/bomb.bz2", &bz_bomb()).await;
        let v = extract(&f, "/uploads/bomb.bz2", None).await;
        session_refusal(&v, 1000);
        assert_eq!(read(&f, "/uploads/bomb").await, None);
        assert_eq!(allowance(&f), 1000);
        assert_extraction_bounded(1000);
    }

    #[tokio::test]
    async fn e2e_0083_040_output_exactly_equal_to_the_remainin() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        limit_remaining(&f, 16).await;
        let v = extract(&f, "/uploads/data.csv.bz2", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(remaining_storage(&f).await, 0);
    }

    #[tokio::test]
    async fn e2e_0083_041_overwrite_counts_net_growth() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/data.csv", &[b'x'; 40]).await;
        limit_remaining(&f, 5).await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", None, true, None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(remaining_storage(&f).await, 29);
    }

    #[tokio::test]
    async fn e2e_0083_042_the_allowance_is_charged_the_full_ou() {
        let f = fixture_allowance(20).await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        seed(&f, "/uploads/data.csv", &[b'x'; 40]).await;
        let v = extract_with(&f, "/uploads/data.csv.bz2", None, true, None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(allowance(&f), 4);
    }

    #[tokio::test]
    async fn e2e_0083_049_both_limits_at_the_same_byte() {
        let f = fixture_allowance(1000).await;
        seed(&f, "/uploads/bomb.bz2", &bz_bomb()).await;
        limit_remaining(&f, 1000).await;
        let v = extract(&f, "/uploads/bomb.bz2", None).await;
        project_refusal(&v);
        assert_eq!(remaining_storage(&f).await, 1000);
        assert_eq!(allowance(&f), 1000);
    }

    #[tokio::test]
    async fn e2e_0083_053_a_refused_extraction_charges_nothing() {
        let f = fixture_allowance(10).await;
        seed(&f, "/uploads/data.csv.bz2", &bzip2(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.bz2", None).await;
        session_refusal(&v, 10);
        write_small(&f).await;
        assert_eq!(allowance(&f), 7);
    }

    // Bare xz files (SPEC-0082, US-0178).

    fn xz(content: &[u8]) -> Vec<u8> {
        let mut enc = liblzma::write::XzEncoder::new(Vec::new(), 6);
        enc.write_all(content).unwrap();
        enc.finish().unwrap()
    }

    /// An xz file made with the given filter chain and integrity check.
    fn xz_with(
        content: &[u8],
        filters: &liblzma::stream::Filters,
        check: liblzma::stream::Check,
    ) -> Vec<u8> {
        let stream = liblzma::stream::Stream::new_stream_encoder(filters, check).unwrap();
        let mut enc = liblzma::write::XzEncoder::new_stream(Vec::new(), stream);
        enc.write_all(content).unwrap();
        enc.finish().unwrap()
    }

    #[tokio::test]
    async fn e2e_0082_001_xz_file_decompresses_next_to_the_arc() {
        let f = fixture().await;
        let archive = xz(CSV);
        seed(&f, "/uploads/data.csv.xz", &archive).await;
        let v = extract(&f, "/uploads/data.csv.xz", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/uploads", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/uploads/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/data.csv.xz").await, Some(archive));
    }

    #[tokio::test]
    async fn e2e_0082_002_upper_case_suffix() {
        let f = fixture().await;
        seed(&f, "/uploads/DATA.CSV.XZ", &xz(CSV)).await;
        let v = extract(&f, "/uploads/DATA.CSV.XZ", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/DATA.CSV").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0082_003_explicit_existing_destination() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/out", true).await.unwrap();
        seed(&f, "/uploads/data.csv.xz", &xz(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.xz", Some("/out")).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/out", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/out/data.csv").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    #[tokio::test]
    async fn e2e_0082_015_a_file_with_an_x86_branch_filter_dec() {
        use liblzma::stream::{Check, Filters, LzmaOptions};
        let f = fixture().await;
        let mut filters = Filters::new();
        filters.x86();
        filters.lzma2(&LzmaOptions::new_preset(6).unwrap());
        seed(&f, "/uploads/bcj.xz", &xz_with(b"hello", &filters, Check::Crc64)).await;
        let v = extract(&f, "/uploads/bcj.xz", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/uploads", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/uploads/bcj").await.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn e2e_0082_026_xz_of_an_empty_file() {
        let f = fixture().await;
        seed(&f, "/uploads/empty.txt.xz", &xz(b"")).await;
        let v = extract(&f, "/uploads/empty.txt.xz", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/uploads", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 0})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/uploads/empty.txt").await, Some(Vec::new()));
    }

    #[tokio::test]
    async fn e2e_0082_034_accents_and_spaces() {
        let f = fixture().await;
        seed(&f, "/uploads/rapport été.txt.xz", &xz(CSV)).await;
        let v = extract(&f, "/uploads/rapport été.txt.xz", None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(read(&f, "/uploads/rapport été.txt").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0082_035_several_dots_and_no_dot() {
        let f = fixture().await;
        seed(&f, "/uploads/a.b.c.xz", &xz(CSV)).await;
        seed(&f, "/uploads/README.xz", &xz(CSV)).await;
        for p in ["/uploads/a.b.c.xz", "/uploads/README.xz"] {
            let v = extract(&f, p, None).await;
            assert!(v.is_ok(), "{p}: {v:?}");
        }
        assert_eq!(read(&f, "/uploads/a.b.c").await.as_deref(), Some(CSV));
        assert_eq!(read(&f, "/uploads/README").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0082_036_success_is_audited() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.xz", &xz(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.xz", None).await;
        assert!(v.is_ok(), "{v:?}");
        let log = f.state.safety.audit(OWNER, MOUNT);
        let lines: Vec<_> = log.iter().filter(|e| e.op == "extract_archive").collect();
        assert_eq!(lines.len(), 1, "{log:?}");
        assert_eq!(lines[0].path, "/uploads/data.csv.xz");
        assert_eq!(lines[0].detail, "destination=/uploads files_written=1 bytes_written=16");
    }

    #[tokio::test]
    async fn e2e_0082_039_missing_destination_is_created() {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.xz", &xz(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.xz", Some("/new/dir")).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/new/dir", "files_written": 1,
                         "dirs_created": 1, "bytes_written": 16})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/new/dir/data.csv").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0082_040_trailing_slash_on_the_destination() {
        let f = fixture().await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.makedirs("/out", true).await.unwrap();
        seed(&f, "/uploads/data.csv.xz", &xz(CSV)).await;
        let v = extract(&f, "/uploads/data.csv.xz", Some("/out/")).await;
        assert_eq!(v.as_ref().ok().and_then(|v| v["destination"].as_str()), Some("/out"), "{v:?}");
        assert_eq!(read(&f, "/out/data.csv").await.as_deref(), Some(CSV));
    }

    #[tokio::test]
    async fn e2e_0082_056_a_file_with_a_sha_256_integrity_chec() {
        use liblzma::stream::{Check, Filters, LzmaOptions};
        let f = fixture().await;
        let mut filters = Filters::new();
        filters.lzma2(&LzmaOptions::new_preset(6).unwrap());
        seed(&f, "/uploads/sha.xz", &xz_with(b"hello", &filters, Check::Sha256)).await;
        let v = extract(&f, "/uploads/sha.xz", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/uploads", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/uploads/sha").await.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn e2e_0082_057_archive_directly_under_the_project_r() {
        let f = fixture().await;
        seed(&f, "/a.xz", &xz(b"hello")).await;
        let v = extract(&f, "/a.xz", None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": "/", "files_written": 1,
                         "dirs_created": 0, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, "/a").await.as_deref(), Some(&b"hello"[..]));
    }

    // Several streams, padding and corrupt input (SPEC-0082, US-0179).

    fn xz_streams(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| xz(p)).collect()
    }

    const XZ_CORRUPT: &str = "archive is corrupt or not a valid xz file:";

    fn assert_xz_corrupt(v: &Result<Value>) {
        let e = v.as_ref().expect_err("a corrupt file must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
        assert!(e.message.starts_with(XZ_CORRUPT), "{}", e.message);
    }

    async fn refused_xz(f: &Fixture, archive: &str, output: &str) {
        let v = extract(f, archive, None).await;
        assert_xz_corrupt(&v);
        assert_eq!(read(f, output).await, None);
    }

    async fn extracted_xz(archive_bytes: &[u8], archive: &str, output: &str, want: &[u8]) {
        let f = fixture().await;
        seed(&f, archive, archive_bytes).await;
        let v = extract(&f, archive, None).await;
        assert!(v.is_ok(), "{v:?}");
        assert_eq!(v.unwrap_or_default()["bytes_written"], want.len());
        assert_eq!(read(&f, output).await.as_deref(), Some(want));
    }

    #[tokio::test]
    async fn e2e_0082_004_two_xz_streams_back_to_back() {
        extracted_xz(
            &xz_streams(&[b"hello ", b"world"]),
            "/uploads/two.txt.xz",
            "/uploads/two.txt",
            b"hello world",
        )
        .await;
    }

    #[tokio::test]
    async fn e2e_0082_043_three_xz_streams() {
        extracted_xz(
            &xz_streams(&[b"a", b"b", b"c"]),
            "/uploads/three.txt.xz",
            "/uploads/three.txt",
            b"abc",
        )
        .await;
    }

    #[tokio::test]
    async fn e2e_0082_044_an_empty_xz_stream_between_two_other() {
        extracted_xz(
            &xz_streams(&[b"a", b"", b"b"]),
            "/uploads/hole.txt.xz",
            "/uploads/hole.txt",
            b"ab",
        )
        .await;
    }

    #[tokio::test]
    async fn e2e_0082_013_bytes_after_a_valid_stream() {
        let f = fixture().await;
        let mut junk = xz(CSV);
        junk.extend_from_slice(b"JUNK");
        seed(&f, "/uploads/tail.xz", &junk).await;
        refused_xz(&f, "/uploads/tail.xz", "/uploads/tail").await;
    }

    #[tokio::test]
    async fn e2e_0082_032_padding_of_4_bytes_between_two_strea() {
        let mut gap = xz(b"a");
        gap.extend_from_slice(&[0u8; 4]);
        gap.extend_from_slice(&xz(b"b"));
        extracted_xz(&gap, "/uploads/gap.txt.xz", "/uploads/gap.txt", b"ab").await;
    }

    #[tokio::test]
    async fn e2e_0082_033_padding_of_3_bytes_is_refused() {
        let f = fixture().await;
        let mut pad = xz(CSV);
        pad.extend_from_slice(&[0u8; 3]);
        seed(&f, "/uploads/pad3.xz", &pad).await;
        refused_xz(&f, "/uploads/pad3.xz", "/uploads/pad3").await;
    }

    #[tokio::test]
    async fn e2e_0082_011_truncated_file() {
        let f = fixture().await;
        let whole = xz(CSV);
        seed(&f, "/uploads/cut.xz", &whole[..whole.len() / 2]).await;
        refused_xz(&f, "/uploads/cut.xz", "/uploads/cut").await;
    }

    #[tokio::test]
    async fn e2e_0082_012_text_file_named_xz() {
        let f = fixture().await;
        seed(&f, "/uploads/fake.xz", b"not xz").await;
        refused_xz(&f, "/uploads/fake.xz", "/uploads/fake").await;
    }

    #[tokio::test]
    async fn e2e_0082_014_damaged_checksum() {
        let f = fixture().await;
        let mut bad = xz(CSV);
        // Offset 24 is inside the compressed block of the 16 byte CSV fixture; every
        // single byte flip of this file was measured as refused.
        bad[24] ^= 0xFF;
        seed(&f, "/uploads/bad.xz", &bad).await;
        refused_xz(&f, "/uploads/bad.xz", "/uploads/bad").await;
    }

    #[tokio::test]
    async fn e2e_0082_027_a_0_byte_file_named_zero_xz() {
        let f = fixture().await;
        seed(&f, "/uploads/zero.xz", b"").await;
        refused_xz(&f, "/uploads/zero.xz", "/uploads/zero").await;
    }

    #[tokio::test]
    async fn e2e_0082_028_zero_padding_after_a_stream_is_accep() {
        let mut pad = xz(b"hello");
        pad.extend_from_slice(&[0u8; 4]);
        extracted_xz(&pad, "/uploads/pad.txt.xz", "/uploads/pad.txt", b"hello").await;
    }

    #[tokio::test]
    async fn e2e_0082_052_padding_of_8_bytes_after_a_stream() {
        let mut pad = xz(b"hello");
        pad.extend_from_slice(&[0u8; 8]);
        extracted_xz(&pad, "/uploads/pad8.txt.xz", "/uploads/pad8.txt", b"hello").await;
    }

    /// The LZMA alone format is what `new_multi_decoder` would accept as xz.
    #[tokio::test]
    async fn xz_0082_lzma_alone_file_is_corrupt() {
        use liblzma::stream::{LzmaOptions, Stream};
        let f = fixture().await;
        let stream = Stream::new_lzma_encoder(&LzmaOptions::new_preset(6).unwrap()).unwrap();
        let mut enc = liblzma::write::XzEncoder::new_stream(Vec::new(), stream);
        enc.write_all(b"hello").unwrap();
        seed(&f, "/uploads/old.xz", &enc.finish().unwrap()).await;
        refused_xz(&f, "/uploads/old.xz", "/uploads/old").await;
    }

    /// Check type 0x02 is reserved: liblzma must refuse it rather than decode unverified.
    #[tokio::test]
    async fn xz_0082_unknown_check_type_is_corrupt() {
        use liblzma::stream::{Check, Filters, LzmaOptions};
        let f = fixture().await;
        let mut filters = Filters::new();
        filters.lzma2(&LzmaOptions::new_preset(6).unwrap());
        let mut file = xz_with(b"hello", &filters, Check::Crc32);
        let crc = |bytes: &[u8]| {
            let mut c = flate2::Crc::new();
            c.update(bytes);
            c.sum().to_le_bytes()
        };
        let len = file.len();
        file[7] = 0x02;
        file[len - 3] = 0x02;
        let header_crc = crc(&file[6..8]);
        file[8..12].copy_from_slice(&header_crc);
        let footer_crc = crc(&file[len - 8..len - 2]);
        file[len - 12..len - 8].copy_from_slice(&footer_crc);
        seed(&f, "/uploads/chk.xz", &file).await;
        refused_xz(&f, "/uploads/chk.xz", "/uploads/chk").await;
    }

    // Password, output name, tar rows (SPEC-0082, US-0180).

    async fn xz_password_refused(password: &str) {
        let f = fixture().await;
        seed(&f, "/uploads/data.csv.xz", &xz(CSV)).await;
        let e = extract_with(&f, "/uploads/data.csv.xz", None, false, Some(password)).await;
        let e = e.expect_err("a password must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, NO_PASSWORD);
        assert_eq!(read(&f, "/uploads/data.csv").await, None);
    }

    #[tokio::test]
    async fn e2e_0082_016_password_given() {
        xz_password_refused("s3cret").await;
    }

    #[tokio::test]
    async fn e2e_0082_045_empty_password() {
        xz_password_refused("").await;
    }

    async fn refuses_xz_name(file: &str, shown: &str) {
        let f = fixture().await;
        let path = format!("/uploads/{file}");
        seed(&f, &path, &xz(CSV)).await;
        let e = extract(&f, &path, None).await;
        let e = e.expect_err("a name with no stem must be refused");
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, format!("cannot derive an output file name from '{shown}'"));
        assert!(f.state.safety.audit(OWNER, MOUNT).iter().all(|a| a.op != "extract_archive"));
        let c = f.state.stores.client(MOUNT).await.unwrap();
        let names: Vec<String> =
            c.list_dir("/uploads").await.unwrap().into_iter().map(|n| n.path).collect();
        assert_eq!(names, vec![path.clone()], "{names:?}");
    }

    #[tokio::test]
    async fn e2e_0082_017_file_named_xz() {
        refuses_xz_name(".xz", ".xz").await;
    }

    #[tokio::test]
    async fn e2e_0082_041_file_named_xz() {
        refuses_xz_name("..xz", "..xz").await;
    }

    #[tokio::test]
    async fn e2e_0082_042_upper_case_xz_alone() {
        refuses_xz_name(".XZ", ".XZ").await;
    }

    async fn tar_xz_is_a_tar(archive: &str, folder: &str) {
        let f = fixture().await;
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut h = tar::Header::new_gnu();
            h.set_path("a.txt").unwrap();
            h.set_size(5);
            h.set_cksum();
            b.append(&h, &b"hello"[..]).unwrap();
            b.finish().unwrap();
        }
        // The tar family still decodes with lzma-rs, so the fixture uses it too.
        let mut packed = Vec::new();
        lzma_rs::xz_compress(&mut std::io::Cursor::new(&tar_bytes), &mut packed).unwrap();
        seed(&f, archive, &packed).await;
        let v = extract_with(&f, archive, None, false, None).await;
        assert_eq!(
            v.as_ref().ok(),
            Some(&json!({"destination": folder, "files_written": 1,
                         "dirs_created": 1, "bytes_written": 5})),
            "{v:?}"
        );
        assert_eq!(read(&f, &format!("{folder}/a.txt")).await.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn e2e_0082_005_tar_xz_is_still_a_tar() {
        tar_xz_is_a_tar("/uploads/pack.tar.xz", "/uploads/pack").await;
    }

    #[tokio::test]
    async fn e2e_0082_006_txz_is_still_a_tar() {
        tar_xz_is_a_tar("/uploads/pack.txz", "/uploads/pack").await;
    }

    #[tokio::test]
    async fn e2e_0082_047_tar_xz_is_still_a_tar() {
        tar_xz_is_a_tar("/uploads/PACK.TAR.XZ", "/uploads/PACK").await;
    }

    #[tokio::test]
    async fn e2e_0082_048_another_extension_is_still_refused() {
        let f = fixture().await;
        seed(&f, "/uploads/pack.lz", b"anything").await;
        let e = extract(&f, "/uploads/pack.lz", None).await;
        let e = e.expect_err("an unknown extension must be refused");
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert_eq!(e.message, "unsupported archive extension: '.lz'");
    }
}
