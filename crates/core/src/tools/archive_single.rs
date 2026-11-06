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
    /// Largest number of bytes held at once, which later stories bound.
    pub(crate) high_water: u64,
}

/// Decoder over `bytes`; `GzDecoder` reads the first member only.
fn decoder(format: SingleFormat, bytes: &[u8]) -> Box<dyn Read + '_> {
    match format {
        SingleFormat::Gzip => Box::new(flate2::read::GzDecoder::new(bytes)),
    }
}

/// Read `reader` to its end in `CHUNK` pieces, stopping with an error past
/// `cap` bytes. Time O(n) in the output, memory O(n): the output is held
/// once, the chunk buffer is fixed.
fn read_capped(mut reader: impl Read, cap: u64) -> std::io::Result<CappedRead> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if (bytes.len() + n) as u64 > cap {
            return Err(std::io::Error::other("decoded size is over the cap"));
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

/// Extract the one file held by `normalized` next to it, or into
/// `destination`. Order: output name, decode, destination, collision,
/// admission, then the writes; nothing is created before the decode passed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn extract_single(
    state: &AppState,
    client: &VolumeClient,
    mount_id: &str,
    person: &str,
    normalized: &str,
    destination: Option<&str>,
    format: SingleFormat,
    suffix_len: usize,
) -> Result<Value> {
    let (parent, file_name) = normalized.rsplit_once('/').unwrap_or(("", normalized));
    let name = output_name(file_name, suffix_len)?.to_string();

    let bytes = client.read_bytes(normalized).await?;
    let label = format.label();
    let decoded = tokio::task::spawn_blocking(move || {
        read_capped(decoder(format, &bytes), u64::MAX).map_err(|e| corrupt(label, e))
    })
    .await
    .map_err(|e| ToolError::internal(format!("archive decode task panicked: {e}")))??;

    tracing::debug!(path = %normalized, high_water = decoded.high_water, "single file decoded");

    let dest = match destination {
        Some(d) => state.safety.normalize_path(d)?,
        None if parent.is_empty() => "/".to_string(),
        None => parent.to_string(),
    };
    let out = crate::util::PosixPath::normpath(&format!("{}/{name}", dest.trim_end_matches('/')));
    if client.exists(&out).await? {
        return Err(ToolError::no_clobber(format!(
            "'{out}' already exists; pass overwrite=true to replace it"
        )));
    }

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
}
