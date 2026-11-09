//! Helpers shared by the SPEC-0084 to SPEC-0087 archive tests (SPEC-0084 DEC-008).

use crate::errors::{Result, code};
use crate::tools::admin::test_support::Fixture;
use crate::tools::archive::extract_archive;
use crate::tools::archive_single::LAST_HIGH_WATER;
use serde_json::Value;
use std::io::Write as _;

pub(crate) const MOUNT: &str = "atlas";
pub(crate) const OWNER: &str = "mia@example.com";

/// A project with an empty `/uploads` folder.
pub(crate) async fn fixture() -> Fixture {
    let f = Fixture::with_config(|_| {}).await;
    f.seed_project(MOUNT, OWNER).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/uploads", true).await.unwrap();
    f
}

pub(crate) async fn seed(f: &Fixture, path: &str, content: &[u8]) {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.write_bytes_atomic(path, content).await.unwrap();
}

/// Authorizes then extracts, as both doors do.
pub(crate) async fn extract(
    f: &Fixture,
    path: &str,
    destination: Option<&str>,
    overwrite: bool,
    password: Option<&str>,
) -> Result<Value> {
    f.state.authorize(MOUNT, OWNER).await.unwrap();
    extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password).await
}

pub(crate) async fn read(f: &Fixture, path: &str) -> Option<Vec<u8>> {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    if c.exists(path).await.unwrap() { Some(c.read_bytes(path).await.unwrap()) } else { None }
}

pub(crate) async fn exists(f: &Fixture, path: &str) -> bool {
    f.state.stores.client(MOUNT).await.unwrap().exists(path).await.unwrap()
}

/// The refusal of a damaged archive of `format`.
pub(crate) fn corrupt_refusal(v: &Result<Value>, format: &str) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    let prefix = format!("archive is corrupt or not a valid {format} file:");
    assert!(e.message.starts_with(&prefix), "{}", e.message);
}

/// A project with an empty `/uploads` folder and a session write allowance of `n` bytes.
pub(crate) async fn fixture_allowance(n: i64) -> Fixture {
    let f = Fixture::with_config(move |c| c.safety.write_quota_bytes = n).await;
    f.seed_project(MOUNT, OWNER).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/uploads", true).await.unwrap();
    f
}

/// Give the project exactly `n` bytes of remaining storage.
pub(crate) async fn limit_remaining(f: &Fixture, n: i64) {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used = crate::quota::used_bytes(&c).await.unwrap();
    f.state.admin.set_quota(MOUNT, Some(used + n)).await.unwrap();
}

pub(crate) async fn remaining_storage(f: &Fixture) -> i64 {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let limit = f.state.safety.project_limit(MOUNT).await.unwrap().unwrap();
    limit - crate::quota::used_bytes(&c).await.unwrap()
}

pub(crate) fn allowance(f: &Fixture) -> i64 {
    f.state.safety.session_remaining(OWNER, MOUNT)
}

/// Forget the bytes the last decode held, so a test reads only its own call.
pub(crate) fn held_reset() {
    LAST_HIGH_WATER.with(|c| c.set(None));
}

/// Bytes the last decode held, 0 when the capped decoder never ran.
pub(crate) fn held() -> u64 {
    LAST_HIGH_WATER.with(std::cell::Cell::get).unwrap_or(0)
}

/// Bytes the last decode held; fails when the capped decoder never ran.
pub(crate) fn held_some() -> u64 {
    LAST_HIGH_WATER.with(std::cell::Cell::get).expect("no capped decode ran")
}

/// The decode held at most `max` bytes (the tokio test runtime is single
/// threaded, so the thread local belongs to this test alone).
pub(crate) fn held_at_most(max: u64) {
    let held = held_some();
    assert!(held <= max, "the extraction held {held} bytes, more than {max}");
}

/// The audit lines the extractions of `path` left in the session.
pub(crate) fn extraction_audit_lines(f: &Fixture, path: &str) -> usize {
    let log = f.state.safety.audit(OWNER, MOUNT);
    log.iter().filter(|e| e.op == "extract_archive" && e.path == path).count()
}

pub(crate) fn project_refusal(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
    assert!(e.message.starts_with("project storage quota of"), "{}", e.message);
}

pub(crate) fn session_refusal(v: &Result<Value>, quota: i64) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::WRITE_QUOTA_EXCEEDED, "{e:?}");
    assert_eq!(e.message, format!("session write quota of {quota} bytes exceeded"));
}

/// A tar of `entries` (path, content), regular files only.
pub(crate) fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut b = tar::Builder::new(&mut out);
        for (path, content) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_path(path).unwrap();
            h.set_size(content.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append(&h, *content).unwrap();
        }
        b.finish().unwrap();
    }
    out
}

pub(crate) fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

pub(crate) fn bz2(bytes: &[u8]) -> Vec<u8> {
    let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

pub(crate) fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut e = liblzma::write::XzEncoder::new(Vec::new(), 6);
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

/// A tar of `zeros.bin`, 10 000 000 zero bytes: a small file once compressed.
pub(crate) fn tar_bomb() -> Vec<u8> {
    tar_bytes(&[("zeros.bin", &vec![0u8; 10_000_000])])
}

/// A tar whose first entry is the symbolic link `link_name` to `target`, then `then` as regular files.
pub(crate) fn tar_with_symlink(link_name: &str, target: &str, then: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut b = tar::Builder::new(&mut out);
        let mut h = tar::Header::new_gnu();
        h.set_path(link_name).unwrap();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_link_name(target).unwrap();
        h.set_cksum();
        b.append(&h, std::io::empty()).unwrap();
        for (path, content) in then {
            let mut h = tar::Header::new_gnu();
            h.set_path(path).unwrap();
            h.set_size(content.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append(&h, *content).unwrap();
        }
        b.finish().unwrap();
    }
    out
}

/// One GNU header declaring `declared` bytes, then `content` and nothing after: the entry lies.
pub(crate) fn tar_lying(name: &str, declared: u64, content: &[u8]) -> Vec<u8> {
    let mut h = tar::Header::new_gnu();
    h.set_path(name).unwrap();
    h.set_size(declared);
    h.set_mode(0o644);
    h.set_cksum();
    let mut out = h.as_bytes().to_vec();
    out.extend_from_slice(content);
    out
}

/// `abc` repeated up to `n` bytes.
pub(crate) fn abc_bytes(n: usize) -> Vec<u8> {
    b"abc".iter().copied().cycle().take(n).collect()
}
