//! Helpers shared by the SPEC-0084 to SPEC-0087 archive tests (SPEC-0084 DEC-008).

use crate::errors::{Result, code};
use crate::tools::admin::test_support::Fixture;
use crate::tools::archive::extract_archive;
use serde_json::Value;

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
