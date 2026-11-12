//! SPEC-0090 tests: the password of a RAR version 4 archive with encrypted file names is
//! judged on the first protected block. Archives are slices and bit flips of the committed
//! ones taken at run time, never committed files.

#![cfg(feature = "rar")]

use crate::errors::{Result, code};
use crate::tools::archive_testkit::*;
use serde_json::Value;

const HP4: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4.rar");
const HP4LONG: &[u8] = include_bytes!("../../tests/fixtures/rar/hp4long.rar");
const V4_SECRET: &[u8] = include_bytes!("../../tests/fixtures/rar/v4secret.rar");
const HIDDEN: &[u8] = include_bytes!("../../tests/fixtures/rar/hidden.rar");
const PASSWORD: &str = "s3cret-Ünï";
const LONG: &str = "CorrectHorseBatteryStapleÜnï0123456789ab";
const WRONG: &str =
    "incorrect password for this archive; call again with the correct 'password' parameter";
const CUT: &str = "archive is corrupt or not a valid rar file: unexpected end of archive";
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

fn cut(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, CUT);
}

fn corrupt_not_cut(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    assert!(e.message.starts_with("archive is corrupt or not a valid rar file:"), "{e:?}");
    assert_ne!(e.message, CUT, "a later damage is not a cut");
}

fn wrong_or_cut(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    let is_wrong = e.code == code::PASSWORD_REQUIRED && e.message == WRONG;
    let is_cut = e.code == code::INVALID_ARGUMENT && e.message == CUT;
    assert!(is_wrong || is_cut, "{e:?}");
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

/// Extracts `bytes` with `password` and checks the two files of `hp4.rar` came out.
async fn extracts(stem: &str, bytes: &[u8], password: &str, files: &[(&str, &[u8])]) {
    let f = fixture().await;
    let archive = format!("/uploads/{stem}.rar");
    seed(&f, &archive, bytes).await;
    let v = extract(&f, &archive, None, false, Some(password)).await.unwrap();
    assert_eq!(v["files_written"], files.len(), "{v}");
    let total: usize = files.iter().map(|(_, c)| c.len()).sum();
    assert_eq!(v["bytes_written"], total, "{v}");
    for (name, content) in files {
        assert_eq!(read(&f, &format!("/uploads/{stem}/{name}")).await.as_deref(), Some(*content));
    }
    assert_eq!(extraction_audit_lines(&f, &archive), 1);
}

#[tokio::test]
async fn e2e_0090_001_right_password_extracts() {
    extracts("hp4", HP4, PASSWORD, &[("a.txt", b"hello"), ("b.txt", b"goodbye")]).await;
}

#[tokio::test]
async fn e2e_0090_002_a_40_character_password_extracts() {
    extracts("hp4long", HP4LONG, LONG, &[("a.txt", b"hello")]).await;
}

#[tokio::test]
async fn e2e_0090_010_one_byte_short_of_the_start() {
    refused("hp4t", &HP4[..43], Some("nope"), cut).await;
}

#[tokio::test]
async fn e2e_0090_011_leading_part_alone_or_one_byte_more() {
    for n in [20, 21] {
        for password in ["nope", PASSWORD] {
            refused("hp4l", &HP4[..n], Some(password), cut).await;
        }
    }
}

#[tokio::test]
async fn e2e_0090_012_right_password_cut_before_the_start_i() {
    for n in [30, 43] {
        refused("hp4r", &HP4[..n], Some(PASSWORD), cut).await;
    }
}

#[tokio::test]
async fn e2e_0090_013_right_password_cut_inside_the_first_p() {
    for n in [44, 50, 91] {
        refused("hp4r", &HP4[..n], Some(PASSWORD), cut).await;
    }
}

#[tokio::test]
async fn e2e_0090_016_right_password_a_later_block_damaged() {
    refused("hp4b", &flip(HP4, 140), Some(PASSWORD), corrupt_not_cut).await;
}

#[tokio::test]
async fn e2e_0090_017_right_password_cut_after_the_first_fi() {
    refused("hp4half", &HP4[..108], Some(PASSWORD), cut).await;
}

#[tokio::test]
async fn e2e_0090_018_right_password_cut_inside_the_second() {
    refused("hp4data", &HP4[..190], Some(PASSWORD), cut).await;
}

#[tokio::test]
async fn e2e_0090_019_a_wrong_password_that_reads_as_a_poss() {
    refused("hp4", HP4, Some("wrong135"), cut).await;
}

/// The leading part of `hp4.rar` (marker, main header, the salt) followed by one protected
/// block start of the given kind and declared size, encrypted under `password` so that it
/// decrypts to exactly that start. Sizes up to 16 carry a valid CRC16, so the block is whole
/// and passes; larger sizes run past the end. Only the 16 byte start is present.
fn forged(password: &str, kind: u8, size: u16) -> Vec<u8> {
    use cbc::cipher::block_padding::NoPadding;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};
    let salt: [u8; 8] = HP4[20..28].try_into().unwrap();
    let mut plain = [0u8; 16];
    plain[2] = kind;
    plain[5..7].copy_from_slice(&size.to_le_bytes());
    if (7..=16).contains(&size) {
        let mut crc = flate2::Crc::new();
        crc.update(&plain[2..usize::from(size)]);
        let low = crc.sum().to_le_bytes();
        plain[..2].copy_from_slice(&low[..2]);
    }
    let (key, iv) = crate::tools::archive_rar_crypt::rar3_kdf(password, &salt);
    let _ = cbc::Encryptor::<aes::Aes128>::new((&key).into(), (&iv).into())
        .encrypt_padded_mut::<NoPadding>(&mut plain, 16);
    [&HP4[..28], &plain[..]].concat()
}

/// Both sides of every minimum of `possible_start`: the file and service kinds need 32, the
/// six others 7, and the kinds just outside 0x74 to 0x7b are never a start. A start that
/// cannot be one is WRONG; a possible one is CUT, whole (size up to 16) or running past; the end kind 0x7b is only tried running past, a whole
/// one closes the archive.
#[tokio::test]
async fn e2e_0090_019_start_kind_and_size_boundaries() {
    let cases: [(u8, u16, bool); 19] = [
        (0x74, 31, false),
        (0x74, 16, false),
        (0x74, 7, false),
        (0x74, 32, true),
        (0x7a, 31, false),
        (0x7a, 16, false),
        (0x7a, 32, true),
        (0x75, 6, false),
        (0x75, 7, true),
        (0x75, 16, true),
        (0x75, 31, true),
        (0x79, 6, false),
        (0x79, 7, true),
        (0x7b, 6, false),
        (0x7b, 31, true),
        (0x73, 100, false),
        (0x7c, 100, false),
        (0x00, 100, false),
        (0xff, 100, false),
    ];
    for (kind, size, possible) in cases {
        let check: fn(&Result<Value>) = if possible { cut } else { wrong };
        refused("hp4f", &forged("nope", kind, size), Some("nope"), check).await;
    }
}

/// 296 wrong passwords through the completeness check, the whole judgment, on scoped
/// threads (one key derivation each), then four through the tool with nothing written.
#[tokio::test]
async fn e2e_0090_020_three_hundred_wrong_passwords() {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let passwords: Vec<String> = (0..300).map(|i| format!("wrong{i}")).collect();
    let verdicts: Vec<(usize, &'static str)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let passwords = &passwords;
                s.spawn(move || {
                    (0..300usize)
                        .filter(|i| i % threads == t && ![0, 135, 299].contains(i))
                        .map(|i| {
                            let verdict = match crate::tools::archive_rar::ensure_rar_complete(
                                HP4,
                                Some(&passwords[i]),
                            ) {
                                Err(e)
                                    if e.code == code::PASSWORD_REQUIRED && e.message == WRONG =>
                                {
                                    "wrong"
                                }
                                Err(e) if e.code == code::INVALID_ARGUMENT && e.message == CUT => {
                                    "cut"
                                }
                                _ => "other",
                            };
                            (i, verdict)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(verdicts.len(), 297);
    let other: Vec<_> = verdicts.iter().filter(|(_, v)| *v == "other").collect();
    assert!(other.is_empty(), "neither wrong nor cut: {other:?}");
    let wrongs = verdicts.iter().filter(|(_, v)| *v == "wrong").count();
    // 297 here plus wrong0, wrong135 and wrong299 through the tool: 291 of 300 at least.
    assert!(wrongs + 2 >= 291, "only {wrongs} wrong of 297");
    refused("hp4", HP4, Some("nope"), wrong).await;
    refused("hp4", HP4, Some("wrong0"), wrong_or_cut).await;
    refused("hp4", HP4, Some("wrong135"), cut).await;
    refused("hp4", HP4, Some("wrong299"), wrong_or_cut).await;
}

#[tokio::test]
async fn e2e_0090_021_no_password_unchanged() {
    refused("hp4", HP4, None, needed).await;
    refused("hp4half", &HP4[..108], None, needed).await;
}

#[tokio::test]
async fn e2e_0090_022_empty_password_unchanged() {
    refused("hp4", HP4, Some(""), wrong).await;
    refused("hp4half", &HP4[..108], Some(""), wrong).await;
}

#[tokio::test]
async fn e2e_0090_023_readable_names_encrypted_data_unchang() {
    refused("v4secret", V4_SECRET, Some("nope"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_024_version_5_unchanged() {
    refused("hidden", HIDDEN, Some("nope"), wrong).await;
}

#[tokio::test]
async fn e2e_0090_025_not_a_member_comes_first() {
    const BOB: &str = "bob@example.com";
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    let e = match f.state.authorize(MOUNT, BOB).await {
        Err(e) => e,
        Ok(_) => crate::tools::archive::extract_archive(
            &f.state,
            MOUNT,
            BOB,
            "/uploads/hp4.rar",
            None,
            false,
            Some("nope"),
        )
        .await
        .expect_err("a non member must be refused"),
    };
    assert_eq!(e.code, code::FORBIDDEN);
    assert_eq!(e.message, "'bob@example.com' is not a member of 'atlas'");
    assert!(!exists(&f, "/uploads/hp4").await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
}

#[tokio::test]
async fn e2e_0090_026_wrong_password_before_a_collision() {
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    seed(&f, "/uploads/hp4/a.txt", b"old").await;
    let v = extract(&f, "/uploads/hp4.rar", None, false, Some("nope")).await;
    wrong(&v);
    assert_eq!(read(&f, "/uploads/hp4/a.txt").await.as_deref(), Some(&b"old"[..]));
    assert!(!exists(&f, "/uploads/hp4/b.txt").await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
}

#[tokio::test]
async fn e2e_0090_027_wrong_password_before_the_storage_lim() {
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    limit_remaining(&f, 5).await;
    let allowance_before = allowance(&f);
    let v = extract(&f, "/uploads/hp4.rar", None, false, Some("nope")).await;
    wrong(&v);
    assert_eq!(remaining_storage(&f).await, 5);
    assert_eq!(allowance(&f), allowance_before);
    assert!(!exists(&f, "/uploads/hp4").await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
}

#[tokio::test]
async fn e2e_0090_028_wrong_password_with_a_new_destination() {
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    let v = extract(&f, "/uploads/hp4.rar", Some("/out"), false, Some("nope")).await;
    wrong(&v);
    assert!(!exists(&f, "/out").await);
    assert!(!exists(&f, "/uploads/hp4").await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
}

#[tokio::test]
async fn e2e_0090_029_a_refused_wrong_password_leaves_nothi() {
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used_before = crate::quota::used_bytes(&c).await.unwrap();
    let allowance_before = allowance(&f);
    for _ in 0..2 {
        let v = extract(&f, "/uploads/hp4.rar", None, false, Some("nope")).await;
        wrong(&v);
        assert!(!exists(&f, "/uploads/hp4").await);
        assert_eq!(crate::quota::used_bytes(&c).await.unwrap(), used_before);
        assert_eq!(allowance(&f), allowance_before);
        assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
    }
}

/// The capture records on the thread that took the lock, so the whole body runs on that one
/// thread (as SPEC-0088 E2E-035).
#[test]
fn e2e_0090_030_the_wrong_password_is_never_recorded() {
    use crate::mcp::server::{ExtractArchiveArgs, McpServer};
    use rmcp::handler::server::wrapper::Parameters;

    const GUESS: &str = "s3cret-Üni";
    let _capture = crate::logging::capture::lock_for_test();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let f = fixture().await;
        seed(&f, "/uploads/hp4.rar", HP4).await;
        let server = McpServer::new(f.state.clone(), OWNER.to_string());
        let args = ExtractArchiveArgs {
            mount_id: MOUNT.into(),
            path: "/uploads/hp4.rar".into(),
            destination: None,
            overwrite: false,
            password: Some(GUESS.into()),
        };
        crate::logging::capture::clear();

        let out = server.fs_extract_archive(Parameters(args)).await.unwrap();

        assert_eq!(out.is_error, Some(true), "{out:?}");
        let text = serde_json::to_string(&out).unwrap();
        assert!(text.contains(WRONG), "{text}");
        assert!(!text.contains(GUESS), "{text}");
        let events = crate::logging::capture::events();
        assert!(
            events.iter().any(|e| format!("{e:?}").contains("ERR_PASSWORD_REQUIRED")),
            "{events:?}"
        );
        assert!(!format!("{events:?}").contains(GUESS), "{events:?}");
        let log = f.state.safety.audit(OWNER, MOUNT);
        assert!(!format!("{log:?}").contains(GUESS), "{log:?}");
        assert!(!log.iter().any(|e| e.path == "/uploads/hp4.rar" && e.op == "extract_archive"));
    });
}

#[tokio::test]
async fn e2e_0090_031_overwrite_set_refused_nothing_replace() {
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    seed(&f, "/uploads/hp4/a.txt", b"old").await;
    let v = extract(&f, "/uploads/hp4.rar", None, true, Some("nope")).await;
    wrong(&v);
    assert_eq!(read(&f, "/uploads/hp4/a.txt").await.as_deref(), Some(&b"old"[..]));
    assert!(!exists(&f, "/uploads/hp4/b.txt").await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
}

#[tokio::test]
async fn e2e_0090_036_retry_with_the_right_password_after_a() {
    let f = fixture().await;
    seed(&f, "/uploads/hp4.rar", HP4).await;
    wrong(&extract(&f, "/uploads/hp4.rar", None, false, Some("s3cret-Üni")).await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 0);
    extract(&f, "/uploads/hp4.rar", None, false, Some(PASSWORD)).await.unwrap();
    assert_eq!(read(&f, "/uploads/hp4/a.txt").await.as_deref(), Some(&b"hello"[..]));
    assert_eq!(read(&f, "/uploads/hp4/b.txt").await.as_deref(), Some(&b"goodbye"[..]));
    assert_eq!(extraction_audit_lines(&f, "/uploads/hp4.rar"), 1);
}
