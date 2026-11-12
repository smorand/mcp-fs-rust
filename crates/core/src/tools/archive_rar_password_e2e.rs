//! SPEC-0090 tests: the password of a RAR version 4 archive with encrypted file names is
//! judged on the first protected block. Archives are slices and bit flips of the committed
//! ones taken at run time, never committed files.

#![cfg(feature = "rar")]

use crate::errors::{Result, code};
use crate::tools::archive_testkit::*;
use serde_json::Value;

const HP4: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4.rar");
const HP4LONG: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4long.rar");
const PASSWORD: &str = "s3cret-Ünï";
const LONG: &str = "CorrectHorseBatteryStapleÜnï0123456789ab";
const WRONG: &str =
    "incorrect password for this archive; call again with the correct 'password' parameter";
const NEEDED: &str =
    "password required to extract this archive; call again with the 'password' parameter";

/// `bytes` with bit 0 of byte `k` flipped.
fn flip(bytes: &[u8], k: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[k] ^= 1;
    out
}

fn wrong(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PASSWORD_REQUIRED, "{e:?}");
    assert_eq!(e.message, WRONG);
}

fn needed(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PASSWORD_REQUIRED, "{e:?}");
    assert_eq!(e.message, NEEDED);
}

/// Seeds `bytes` as `/uploads/{stem}.rar`, extracts it and checks the refusal is `check`
/// and that nothing was created, charged or audited.
async fn refused(stem: &str, bytes: &[u8], password: Option<&str>, check: fn(&Result<Value>)) {
    let f = fixture().await;
    let archive = format!("/uploads/{stem}.rar");
    seed(&f, &archive, bytes).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used_before = crate::quota::used_bytes(&c).await.unwrap();
    let remaining_before = f.state.safety.session_remaining(OWNER, MOUNT);

    let v = extract(&f, &archive, None, false, password).await;

    check(&v);
    assert!(!exists(&f, &format!("/uploads/{stem}")).await, "/uploads/{stem} must not exist");
    assert_eq!(crate::quota::used_bytes(&c).await.unwrap(), used_before);
    assert_eq!(f.state.safety.session_remaining(OWNER, MOUNT), remaining_before);
    assert_eq!(extraction_audit_lines(&f, &archive), 0);
}

#[tokio::test]
async fn e2e_0090_003_wrong_password_on_the_whole_archive() {
    refused("hp4", HP4, Some("nope"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_004_wrong_password_that_today_reads_as_a() {
    refused("hp4", HP4, Some("w785"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_005_password_one_letter_off() {
    refused("hp4", HP4, Some("s3cret-Üni"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_006_another_archive_s_password() {
    refused("hp4long", HP4LONG, Some(PASSWORD), wrong).await;
}

#[tokio::test]
async fn e2e_0090_007_wrong_password_on_a_cut_archive() {
    for password in ["nope", "w785"] {
        refused("hp4half", &HP4[..108], Some(password), wrong).await;
    }
}

#[tokio::test]
async fn e2e_0090_008_wrong_password_archive_cut_inside_the() {
    refused("hp4in", &HP4[..50], Some("nope"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_009_start_exactly_whole() {
    refused("hp4s", &HP4[..44], Some("nope"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_014_right_password_first_protected_block() {
    refused("hp4d", &flip(HP4, 50), Some(PASSWORD), wrong).await;
}

#[tokio::test]
async fn e2e_0090_015_wrong_password_on_the_damaged_archive() {
    refused("hp4d", &flip(HP4, 50), Some("nope"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_035_right_password_first_block_of_another() {
    refused("hp4longd", &flip(HP4LONG, 50), Some(LONG), wrong).await;
}

#[tokio::test]
async fn e2e_0090_mod_002_a_damaged_first_block_with_the_ri() {
    let damaged = flip(&HP4[..108], 50);
    refused("hp4dc", &damaged, Some(PASSWORD), wrong).await;
    refused("hp4dc", &damaged, None, needed).await;
    refused("hp4dc", &damaged, Some(""), wrong).await;
}
