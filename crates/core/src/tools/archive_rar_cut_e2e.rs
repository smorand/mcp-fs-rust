//! SPEC-0087 tests: a RAR archive cut anywhere after its marker is refused with one
//! message, and nothing is written. Cuts are slices of the committed archives taken at
//! run time, never committed files.

#[cfg(feature = "rar")]
use crate::errors::Result;
use crate::errors::code;
use crate::tools::archive_testkit::*;
#[cfg(feature = "rar")]
use serde_json::Value;

#[cfg(feature = "rar")]
const V4: &[u8] = include_bytes!("../../tests/fixtures/rar/v4.rar");
#[cfg(feature = "rar")]
const V5: &[u8] = include_bytes!("../../tests/fixtures/rar/v5.rar");
const TWO4: &[u8] = include_bytes!("../../tests/fixtures/rar/two4.rar");
#[cfg(feature = "rar")]
const V4_SECRET: &[u8] = include_bytes!("../../tests/fixtures/rar/v4secret.rar");
#[cfg(feature = "rar")]
const PASSWORD: &str = "s3cret-Ünï";
#[cfg(feature = "rar")]
const TRUNCATED: &str = "archive is corrupt or not a valid rar file: unexpected end of archive";

#[cfg(feature = "rar")]
/// The refusal every cut after a whole marker gets.
fn truncated_refusal(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, TRUNCATED);
}

#[cfg(feature = "rar")]
/// Seeds `bytes` as `/uploads/{stem}.rar`, extracts it and checks the refusal is `check`
/// and that nothing was created, charged or audited.
async fn assert_refused_and_nothing_written(
    stem: &str,
    bytes: &[u8],
    password: Option<&str>,
    check: fn(&Result<Value>),
) {
    let f = fixture().await;
    let archive = format!("/uploads/{stem}.rar");
    seed(&f, &archive, bytes).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used_before = crate::quota::used_bytes(&c).await.unwrap();

    let v = extract(&f, &archive, None, false, password).await;

    check(&v);
    assert!(!exists(&f, &format!("/uploads/{stem}")).await, "/uploads/{stem} must not exist");
    assert_eq!(crate::quota::used_bytes(&c).await.unwrap(), used_before);
    assert_eq!(extraction_audit_lines(&f, &archive), 0);
}

#[cfg(feature = "rar")]
async fn assert_truncated(stem: &str, bytes: &[u8], password: Option<&str>) {
    assert_refused_and_nothing_written(stem, bytes, password, truncated_refusal).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_008_version_4_cut_between_two_files() {
    assert_truncated("half4", &TWO4[..67], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_009_version_4_missing_only_its_end_of_ar() {
    assert_truncated("noend4", &V4[..67], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_011_version_4_cut_inside_a_header() {
    assert_truncated("hdr4", &TWO4[..90], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_012_version_4_marker_and_main_header_onl() {
    assert_truncated("main4", &V4[..20], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_013_version_4_marker_alone() {
    assert_truncated("mark4", &V4[..7], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_014_cut_inside_the_marker() {
    let corrupt = |v: &Result<Value>| corrupt_refusal(v, "rar");
    assert_refused_and_nothing_written("m6", &V4[..6], None, corrupt).await;
    assert_refused_and_nothing_written("m7", &V5[..7], None, corrupt).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_015_one_byte_short() {
    assert_truncated("short4", &V4[..73], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_019_version_4_encrypted_data_cut_right_p() {
    assert_truncated("v4cut", &V4_SECRET[..78], Some(PASSWORD)).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_020_version_4_encrypted_data_cut_no_pass() {
    assert_truncated("v4cut", &V4_SECRET[..78], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_021_version_4_encrypted_data_cut_wrong_p() {
    assert_truncated("v4cut", &V4_SECRET[..78], Some("nope")).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_036_version_5_missing_only_its_end_of_ar() {
    assert_truncated("noend5", &V5[..68], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_047_bytes_before_the_marker_cut_archive() {
    let prefixed = |archive: &[u8]| [&[0u8; 100][..], archive].concat();
    assert_truncated("sfxcut4", &prefixed(&TWO4[..67]), None).await;
    assert_truncated("sfxcut5", &prefixed(&V5[..68]), None).await;
}

#[cfg(feature = "rar")]
const DIR4: &[u8] = include_bytes!("../../tests/fixtures/rar/dir4.rar");
#[cfg(feature = "rar")]
const SOLID4: &[u8] = include_bytes!("../../tests/fixtures/rar/solid4.rar");
#[cfg(feature = "rar")]
const MULTI_VOLUME: &str =
    "multi volume RAR archives are not supported: extract a single file archive";

/// Extracts `/uploads/{stem}.rar` and checks the answer counts and the files read back.
#[cfg(feature = "rar")]
async fn assert_extracts(stem: &str, bytes: &[u8], files: &[(&str, &[u8])]) {
    let f = fixture().await;
    let archive = format!("/uploads/{stem}.rar");
    seed(&f, &archive, bytes).await;
    let v = extract(&f, &archive, None, false, None).await.unwrap();
    assert_eq!(v["files_written"], files.len(), "{v}");
    let total: usize = files.iter().map(|(_, c)| c.len()).sum();
    assert_eq!(v["bytes_written"], total, "{v}");
    for (name, content) in files {
        assert_eq!(read(&f, &format!("/uploads/{stem}/{name}")).await.as_deref(), Some(*content));
    }
    assert_eq!(extraction_audit_lines(&f, &archive), 1);
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_001_complete_version_4_archive_with_two() {
    assert_extracts("two4", TWO4, &[("a.txt", b"hello"), ("b.txt", b"goodbye")]).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_005_complete_empty_version_4_archive() {
    let empty4 = [&V4[..20], &V4[V4.len() - 7..]].concat();
    let f = fixture().await;
    seed(&f, "/uploads/empty4.rar", &empty4).await;
    let v = extract(&f, "/uploads/empty4.rar", None, false, None).await.unwrap();
    assert_eq!(v["files_written"], 0, "{v}");
    assert_eq!(v["bytes_written"], 0, "{v}");
    assert_eq!(v["dirs_created"], 1, "{v}");
    assert!(exists(&f, "/uploads/empty4").await);
    let c = f.state.stores.client(MOUNT).await.unwrap();
    assert!(c.list_dir("/uploads/empty4").await.unwrap().is_empty());
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_006_bytes_after_the_end_of_archive_block() {
    let tail = |a: &[u8]| [a, b"trailing junk"].concat();
    assert_extracts("tail4", &tail(V4), &[("a.txt", b"hello")]).await;
    assert_extracts("tail5", &tail(V5), &[("a.txt", b"hello")]).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_007_bytes_before_the_marker_complete_arc() {
    let sfx = |a: &[u8]| [&[0u8; 100][..], a].concat();
    assert_extracts("sfx4", &sfx(V4), &[("a.txt", b"hello")]).await;
    assert_extracts("sfx5", &sfx(V5), &[("a.txt", b"hello")]).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_017_version_4_cut_after_a_folder() {
    assert_truncated("dircut", &DIR4[..107], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_018_solid_version_4_cut_between_files() {
    // 80 is where `a.txt`'s block ends (the walk must see a whole block then no more); 79, the
    // offset the spec measured, cuts its last data byte.
    assert_truncated("solidcut", &SOLID4[..80], None).await;
    assert_truncated("solidcut79", &SOLID4[..79], None).await;
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_041_a_multi_volume_name_wins_over_a_cut() {
    let f = fixture().await;
    seed(&f, "/uploads/big.part1.rar", &TWO4[..67]).await;
    let v = extract(&f, "/uploads/big.part1.rar", None, false, None).await;
    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::NOT_SUPPORTED);
    assert_eq!(e.message, MULTI_VOLUME);
    assert!(!exists(&f, "/uploads/big.part1").await);
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_042_membership_wins_over_a_cut() {
    const BOB: &str = "bob@example.com";
    let f = fixture().await;
    seed(&f, "/uploads/half4.rar", &TWO4[..67]).await;
    let e = match f.state.authorize(MOUNT, BOB).await {
        Err(e) => e,
        Ok(_) => crate::tools::archive::extract_archive(
            &f.state,
            MOUNT,
            BOB,
            "/uploads/half4.rar",
            None,
            false,
            None,
        )
        .await
        .expect_err("a non member must be refused"),
    };
    assert_eq!(e.code, code::FORBIDDEN);
    assert_eq!(e.message, "'bob@example.com' is not a member of 'atlas'");
    assert!(!exists(&f, "/uploads/half4").await);
}

#[cfg(not(feature = "rar"))]
#[tokio::test]
async fn e2e_0087_043_server_without_rar_support() {
    let f = fixture().await;
    seed(&f, "/uploads/half4.rar", &TWO4[..67]).await;
    let e = extract(&f, "/uploads/half4.rar", None, false, None).await.unwrap_err();
    assert_eq!(e.code, code::NOT_SUPPORTED);
    assert_eq!(e.message, "unsupported archive extension: '.rar'");
    assert!(!exists(&f, "/uploads/half4").await);
}

#[cfg(feature = "rar")]
#[tokio::test]
async fn e2e_0087_054_a_cut_with_a_destination_inside_the() {
    let f = fixture().await;
    seed(&f, "/uploads/half4.rar", &TWO4[..67]).await;
    let v = extract(&f, "/uploads/half4.rar", Some("out"), false, None).await;
    truncated_refusal(&v);
    assert!(!exists(&f, "/out").await);
}
