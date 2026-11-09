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

const BROKEN: &[u8] = b"not a zip";

#[tokio::test]
async fn e2e_0086_019_the_destination_wins_over_a_damaged() {
    let f = fixture().await;
    seed(&f, "/uploads/broken.zip", BROKEN).await;

    let v = extract(&f, "/uploads/broken.zip", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/broken.zip", &["/x", "/uploads/broken"]).await;
}

#[tokio::test]
async fn e2e_0086_020_the_destination_wins_over_the_limits() {
    let f = fixture_allowance(1000).await;
    seed(&f, "/uploads/bomb.tar.gz", &gz(&tar_bomb())).await;
    held_reset();

    let v = extract(&f, "/uploads/bomb.tar.gz", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/bomb.tar.gz", &["/x", "/uploads/bomb"]).await;
    assert_eq!(allowance(&f), 1000);
    assert_eq!(held(), 0);
}

#[tokio::test]
async fn e2e_0086_021_the_destination_wins_over_a_link_ent() {
    let f = fixture().await;
    let zip = crate::tools::archive::tests::build_zip_with_special_entry(
        "ok.txt",
        HELLO,
        "l",
        b"/etc/passwd",
        true,
    );
    seed(&f, "/uploads/link.zip", &zip).await;

    let v = extract(&f, "/uploads/link.zip", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/link.zip", &["/x", "/uploads/link"]).await;
}

#[tokio::test]
async fn e2e_0086_022_the_destination_wins_over_an_escapin() {
    let f = fixture().await;
    seed(
        &f,
        "/uploads/evil.tar",
        &crate::tools::archive::tests::build_tar(&[("../../etc/passwd", b"pwned")]),
    )
    .await;

    let v = extract(&f, "/uploads/evil.tar", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/evil.tar", &["/x", "/etc/passwd"]).await;
}

#[tokio::test]
async fn e2e_0086_023_the_destination_wins_over_a_missing() {
    let f = fixture().await;
    let sz = crate::tools::archive::tests::build_sevenz_ext(
        &[("s.txt", HELLO, None)],
        Some("s3cret"),
        false,
    );
    seed(&f, "/uploads/secret.7z", &sz).await;

    let v = extract(&f, "/uploads/secret.7z", Some("../../x"), false, None).await;

    climb_refusal(&v, "../../x");
    assert_nothing_written(&f, "/uploads/secret.7z", &["/x", "/uploads/secret"]).await;
}

#[tokio::test]
async fn e2e_0086_024_membership_wins_over_the_destination() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.zip", &build_zip(&[("a.txt", HELLO)], None)).await;

    let r = f.state.authorize(MOUNT, "eve@example.com").await;

    let e = r.expect_err("a non member is refused");
    assert_eq!(e.code, code::FORBIDDEN, "{e:?}");
    assert_eq!(e.message, "'eve@example.com' is not a member of 'atlas'");
}

#[tokio::test]
async fn e2e_0086_025_a_missing_path_wins_over_the_destina() {
    let f = fixture().await;

    let v = extract(&f, "/uploads/none.zip", Some("../../x"), false, None).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::NOT_FOUND, "{e:?}");
    assert_eq!(e.message, "'/uploads/none.zip' not found");
}

#[tokio::test]
async fn e2e_0086_026_a_folder_path_wins_over_the_destinat() {
    let f = fixture().await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/uploads/dir.zip", true).await.unwrap();

    let v = extract(&f, "/uploads/dir.zip", Some("../../x"), false, None).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, "'/uploads/dir.zip' is a directory, not an archive file");
}

#[tokio::test]
async fn e2e_0086_027_an_unsupported_extension_wins_over_t() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.lz", b"12345").await;

    let v = extract(&f, "/uploads/pack.lz", Some("../../x"), false, None).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::NOT_SUPPORTED, "{e:?}");
    assert_eq!(e.message, "unsupported archive extension: '.lz'");
}

#[tokio::test]
async fn e2e_0086_028_a_password_on_a_tar_archive_wins_ove() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.gz", &gz(&tar_bytes(&[("a.txt", HELLO)]))).await;

    let v = extract(&f, "/uploads/pack.tar.gz", Some("../../x"), false, Some("x")).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, "password is not applicable to this archive format");
}

#[tokio::test]
async fn e2e_0086_030_a_nul_character_is_refused_before_th() {
    let f = fixture().await;
    seed(&f, "/uploads/broken.zip", BROKEN).await;

    let v = extract(&f, "/uploads/broken.zip", Some("out\0x"), false, None).await;

    nul_refusal(&v);
    assert_nothing_written(&f, "/uploads/broken.zip", &["/out"]).await;
}

#[tokio::test]
async fn e2e_0086_036_a_nul_character_in_an_otherwise_vali() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.7z", &sevenz_solid(&[("a.txt", HELLO)])).await;

    let v = extract(&f, "/uploads/pack.7z", Some("out\0x"), false, None).await;

    nul_refusal(&v);
    assert_nothing_written(&f, "/uploads/pack.7z", &["/out"]).await;
}

fn nul_refusal(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS, "{e:?}");
    assert_eq!(e.message, "path contains a NUL byte");
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

    #[tokio::test]
    async fn e2e_0086_029_a_multi_volume_rar_name_wins_over_th() {
        let f = fixture().await;
        seed(&f, "/uploads/pack.part1.rar", V5).await;

        let v = extract(&f, "/uploads/pack.part1.rar", Some("../../x"), false, None).await;

        let e = v.expect_err("the extraction must be refused");
        assert_eq!(e.code, code::NOT_SUPPORTED, "{e:?}");
        assert_eq!(
            e.message,
            "multi volume RAR archives are not supported: extract a single file archive"
        );
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
