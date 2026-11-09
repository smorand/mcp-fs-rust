//! SPEC-0087 tests: a RAR archive cut anywhere after its marker is refused with one
//! message, and nothing is written. Cuts are slices of the committed archives taken at
//! run time, never committed files.

use crate::errors::{Result, code};
use crate::tools::archive_testkit::*;
use serde_json::Value;

const V4: &[u8] = include_bytes!("../../tests/fixtures/rar/v4.rar");
const V5: &[u8] = include_bytes!("../../tests/fixtures/rar/v5.rar");
const TWO4: &[u8] = include_bytes!("../../tests/fixtures/rar/two4.rar");
const V4_SECRET: &[u8] = include_bytes!("../../tests/fixtures/rar/v4secret.rar");
const PASSWORD: &str = "s3cret-Ünï";
const TRUNCATED: &str = "archive is corrupt or not a valid rar file: unexpected end of archive";

/// The refusal every cut after a whole marker gets.
fn truncated_refusal(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, TRUNCATED);
}

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
