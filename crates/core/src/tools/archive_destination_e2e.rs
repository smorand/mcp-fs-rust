//! SPEC-0086 tests: one destination rule for every archive format. A climb
//! above the project root is refused with the destination as typed; a name
//! made of dots and letters is a folder.

use crate::errors::{Result, code};
use crate::tools::archive::tests::build_zip;
use crate::tools::archive_testkit::*;
use serde_json::Value;

const HELLO: &[u8] = b"hello";

/// The refusal of a destination that climbs out of the project.
fn climb_refusal(v: &Result<Value>, typed: &str) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS, "{e:?}");
    assert_eq!(e.message, format!("path escapes the volume root: {typed}"));
}

/// Nothing created, charged or audited by the refused call of `archive`.
async fn assert_nothing_written(
    f: &crate::tools::admin::test_support::Fixture,
    archive: &str,
    paths: &[&str],
) {
    for p in paths {
        assert!(!exists(f, p).await, "{p} must not exist");
    }
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used_before_extraction = crate::quota::used_bytes(&c).await.unwrap();
    let archive_len = read(f, archive).await.unwrap().len() as i64;
    assert_eq!(used_before_extraction, archive_len, "only the archive is stored");
    assert_eq!(extraction_audit_lines(f, archive), 0);
}

#[tokio::test]
async fn e2e_0086_009_one_step_above_the_root() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.zip", &build_zip(&[("a.txt", HELLO)], None)).await;

    let v = extract(&f, "/uploads/pack.zip", Some(".."), false, None).await;

    climb_refusal(&v, "..");
    assert_nothing_written(&f, "/uploads/pack.zip", &["/a.txt"]).await;
}

#[tokio::test]
async fn e2e_0086_010_names_made_of_dots_and_letters_are_f() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.xz", &xz(&tar_bytes(&[("a.txt", HELLO)]))).await;

    let v = extract(&f, "/uploads/pack.tar.xz", Some("..out"), false, None).await.unwrap();
    assert_eq!(v["destination"], "/..out");
    let v = extract(&f, "/uploads/pack.tar.xz", Some("out.."), false, None).await.unwrap();
    assert_eq!(v["destination"], "/out..");

    assert_eq!(read(&f, "/..out/a.txt").await.unwrap(), HELLO);
    assert_eq!(read(&f, "/out../a.txt").await.unwrap(), HELLO);
}

#[tokio::test]
async fn e2e_0086_011_zip_relative_destination_outside_the() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.zip", &build_zip(&[("a.txt", HELLO)], None)).await;

    let v = extract(&f, "/uploads/pack.zip", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/pack.zip", &["/x", "/x/a.txt"]).await;
}

#[tokio::test]
async fn e2e_0086_012_7z_relative_destination_outside_the() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.7z", &sevenz_solid(&[("a.txt", HELLO)])).await;

    let v = extract(&f, "/uploads/pack.7z", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/pack.7z", &["/x", "/x/a.txt"]).await;
}

#[tokio::test]
async fn e2e_0086_013_plain_tar_relative_destination_outsi() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar", &tar_bytes(&[("a.txt", HELLO)])).await;

    let v = extract(&f, "/uploads/pack.tar", Some("../x"), false, None).await;

    climb_refusal(&v, "../x");
    assert_nothing_written(&f, "/uploads/pack.tar", &["/x", "/x/a.txt"]).await;
}

#[tokio::test]
async fn e2e_0086_014_climb_revealed_only_after_resolving() {
    let f = fixture().await;
    let tar = tar_bytes(&[("a.txt", HELLO)]);
    seed(&f, "/uploads/pack.tgz", &gz(&tar)).await;
    seed(&f, "/uploads/pack.tar.bz2", &bz2(&tar)).await;
    seed(&f, "/uploads/pack.TAR.XZ", &xz(&tar)).await;

    for archive in ["/uploads/pack.tgz", "/uploads/pack.tar.bz2", "/uploads/pack.TAR.XZ"] {
        let v = extract(&f, archive, Some("a/../../x"), false, None).await;

        climb_refusal(&v, "a/../../x");
        assert!(!exists(&f, "/a").await, "{archive}");
        assert!(!exists(&f, "/x").await, "{archive}");
        assert_eq!(extraction_audit_lines(&f, archive), 0, "{archive}");
    }
}

#[tokio::test]
async fn e2e_0086_015_zip_absolute_destination_outside_the() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.zip", &build_zip(&[("a.txt", HELLO)], None)).await;

    let v = extract(&f, "/uploads/pack.zip", Some("/../x"), false, None).await;

    climb_refusal(&v, "/../x");
    assert_nothing_written(&f, "/uploads/pack.zip", &["/x", "/x/a.txt"]).await;
}

#[tokio::test]
async fn e2e_0086_016_bare_gzip_file_absolute_destination() {
    let f = fixture().await;
    seed(&f, "/uploads/a.txt.gz", &gz(HELLO)).await;

    let v = extract(&f, "/uploads/a.txt.gz", Some("/../x"), false, None).await;

    climb_refusal(&v, "/../x");
    assert_nothing_written(&f, "/uploads/a.txt.gz", &["/x", "/x/a.txt"]).await;
}

#[tokio::test]
async fn e2e_0086_018_bare_bzip2_and_xz_files_absolute_des() {
    let f = fixture().await;
    seed(&f, "/uploads/a.txt.bz2", &bz2(HELLO)).await;
    seed(&f, "/uploads/a.txt.xz", &xz(HELLO)).await;

    for archive in ["/uploads/a.txt.bz2", "/uploads/a.txt.xz"] {
        let v = extract(&f, archive, Some("/../x"), false, None).await;

        climb_refusal(&v, "/../x");
        assert!(!exists(&f, "/x").await, "{archive}");
        assert_eq!(extraction_audit_lines(&f, archive), 0, "{archive}");
    }
}

#[tokio::test]
async fn e2e_0086_035_overwrite_does_not_lift_the_refusal() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.zip", &build_zip(&[("a.txt", HELLO)], None)).await;
    seed(&f, "/x/a.txt", b"old").await;

    let v = extract(&f, "/uploads/pack.zip", Some("../../x"), true, None).await;

    climb_refusal(&v, "../../x");
    assert_eq!(read(&f, "/x/a.txt").await.unwrap(), b"old");
}

/// RAR tests sit in a module whose path contains `archive_rar`, the filter
/// `test.sh` uses for the feature run (SPEC-0085 DEC-011).
#[cfg(feature = "rar")]
mod archive_rar {
    use super::*;

    const V5: &[u8] = include_bytes!("../../tests/fixtures/rar/v5.rar");

    #[tokio::test]
    async fn e2e_0086_017_rar_absolute_climb_through_a_folder() {
        let f = fixture().await;
        seed(&f, "/uploads/pack.rar", V5).await;

        let v = extract(&f, "/uploads/pack.rar", Some("/a/../../b"), false, None).await;

        climb_refusal(&v, "/a/../../b");
        assert_nothing_written(&f, "/uploads/pack.rar", &["/b", "/a"]).await;
    }
}

#[test]
fn resolve_destination_refuses_a_nul_byte_first() {
    let e = crate::tools::archive_single::resolve_destination("a\0/../..").unwrap_err();
    assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS);
    assert_eq!(e.message, "path contains a NUL byte");
}

#[test]
fn resolve_destination_counts_each_step_up_once() {
    let r = crate::tools::archive_single::resolve_destination;
    assert_eq!(r("a/b/../c/../../x").unwrap(), "/x");
    assert_eq!(r("./a//b/..").unwrap(), "/a");
    assert!(r("a/b/../../../x").is_err());
}
