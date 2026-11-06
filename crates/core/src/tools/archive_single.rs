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
}

impl SingleFormat {
    /// Name used in the corrupt archive message.
    fn label(self) -> &'static str {
        match self {
            Self::Gzip => "gz",
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
}

#[cfg(test)]
thread_local! {
    /// Largest buffer the last decode of this thread held, so a test can
    /// measure the decoder inside a whole extraction.
    pub(crate) static LAST_HIGH_WATER: std::cell::Cell<Option<u64>> =
        const { std::cell::Cell::new(None) };
}

/// Decoder over `bytes`; `MultiGzDecoder` reads every member back to back
/// and fails on bytes that are not a gzip member, where `GzDecoder` would stop
/// after the first member and ignore the rest.
fn decoder(format: SingleFormat, bytes: &[u8]) -> Box<dyn Read + '_> {
    match format {
        SingleFormat::Gzip => Box::new(flate2::read::MultiGzDecoder::new(bytes)),
    }
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
        Some(d) => state.safety.normalize_path(d)?,
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
    let read = tokio::task::spawn_blocking(move || read_capped(decoder(format, &bytes), cap))
        .await
        .map_err(|e| ToolError::internal(format!("archive decode task panicked: {e}")))?;
    let decoded = match read {
        Ok(decoded) => decoded,
        Err(ReadError::Io(e)) => return Err(corrupt(label, e)),
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
        let err = super::read_capped(super::decoder(super::SingleFormat::Gzip, &bomb()), cap)
            .expect_err("the bomb is over the cap");
        match err {
            super::ReadError::CapExceeded { high_water } => {
                assert!(high_water <= cap + 65_536, "held {high_water} bytes");
                assert!(high_water < 1_000_000, "held {high_water} bytes");
            }
            super::ReadError::Io(e) => panic!("expected the cap error, got {e}"),
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
}
