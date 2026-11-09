//! SPEC-0085 exploit tests: declared archive sizes are counted exactly, never
//! as a negative amount (E2E-SEC-001..004). Each archive carries its payload in
//! a hand built zip64 header, and each test checks a second channel: the
//! stored bytes, the session allowance, what another member sees.

use crate::core::fs_ops;
use crate::errors::code;
use crate::tools::admin::test_support::Fixture;
use crate::tools::archive_testkit::*;

/// 2^63: the first size a signed cast turns negative.
const TWO_63: u64 = 9_223_372_036_854_775_808;

fn free_zip() -> Vec<u8> {
    zip_declaring(&[("free.bin", TWO_63, &vec![0u8; 10_000_000])])
}

async fn used(f: &Fixture) -> i64 {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    crate::quota::used_bytes(&c).await.unwrap()
}

async fn write(f: &Fixture, path: &str, n: usize) -> crate::errors::Result<serde_json::Value> {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    fs_ops::write_bytes(&c, &f.state.safety, OWNER, MOUNT, path, &vec![b'w'; n], true, true).await
}

#[tokio::test]
async fn e2e_0085_sec_001_size_of_2_63_against_the_storage_lim() {
    let f = fixture().await;
    seed(&f, "/uploads/free.zip", &free_zip()).await;
    limit_mb_remaining(&f, 1_000).await;

    let v = extract(&f, "/uploads/free.zip", None, false, None).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
    assert_eq!(
        e.message,
        "project storage quota of 1 MB exceeded: 1047576 bytes used, 9223372036854775807 bytes requested"
    );
    assert_eq!(used(&f).await, 1_047_576);
    let noa = add_second_member(&f).await;
    f.state.authorize(MOUNT, noa).await.unwrap();
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let listing = fs_ops::list_dir(&c, "/uploads", true, "name", false).await.unwrap();
    assert!(!listing.to_string().contains("\"free\""), "{listing}");
    assert!(!exists(&f, "/uploads/free/free.bin").await);
}

#[tokio::test]
async fn e2e_0085_sec_002_size_of_2_63_against_the_session_all() {
    let f = fixture_allowance(1_000).await;
    seed(&f, "/uploads/free.zip", &free_zip()).await;

    let v = extract(&f, "/uploads/free.zip", None, false, None).await;

    session_refusal(&v, 1_000);
    session_refusal(&write(&f, "/uploads/w.bin", 1_001).await, 1_000);
    write(&f, "/uploads/w.bin", 1_000).await.expect("the allowance is whole");
    assert!(!exists(&f, "/uploads/free/free.bin").await);
}

#[tokio::test]
async fn e2e_0085_sec_003_the_largest_declarable_size() {
    let f = fixture().await;
    seed(&f, "/uploads/max.zip", &zip_declaring(&[("a.txt", u64::MAX, b"hello")])).await;
    limit_mb_remaining(&f, 1_000).await;

    let v = extract(&f, "/uploads/max.zip", None, false, None).await;

    project_refusal(&v);
    let e = v.unwrap_err();
    assert!(e.message.ends_with("9223372036854775807 bytes requested"), "{}", e.message);
    assert_eq!(used(&f).await, 1_047_576);
    assert!(!exists(&f, "/uploads/max/a.txt").await);
}

#[tokio::test]
async fn e2e_0085_sec_004_two_sizes_whose_sum_wraps() {
    let f = fixture().await;
    let max = i64::MAX as u64;
    seed(&f, "/uploads/wrap.zip", &zip_declaring(&[("a.txt", max, b"a"), ("b.txt", max, b"b")]))
        .await;

    let v = extract(&f, "/uploads/wrap.zip", None, false, None).await;

    session_refusal(&v, 52_428_800);
    write(&f, "/uploads/w.bin", 52_428_800).await.expect("nothing was charged");
    assert!(!exists(&f, "/uploads/wrap").await);
}

/// SPEC-0085 DEC-003 at the admission itself: two sizes of `i64::MAX` sum to
/// more than any limit even for a caller that bypasses the archive's own
/// saturation, so the net change never wraps negative and skips the check.
#[tokio::test]
async fn e2e_0085_sec_admit_paths_sum_saturates() {
    let f = fixture().await;
    limit_mb_remaining(&f, 1_000).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let paths = [("/a".to_string(), i64::MAX), ("/b".to_string(), i64::MAX)];

    let e = crate::quota::admit_paths(&c, &f.state.safety, OWNER, MOUNT, 0, &paths).await.err();

    let e = e.expect("the admission must be refused");
    assert_eq!(e.code, code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
    assert!(e.message.ends_with("9223372036854775807 bytes requested"), "{}", e.message);
    assert_eq!(used(&f).await, 1_047_576);
}

/// 7-Zip's unix extension flag with a symlink mode in the high 16 bits.
const SEVENZ_LINK: u32 = 0x8000 | (0o120_777 << 16);

fn mid_zip(name: &str, n: usize) -> Vec<u8> {
    crate::tools::archive::tests::build_zip(&[(name, &abc_bytes(n))], None)
}

#[tokio::test]
async fn e2e_0085_001_zip_within_the_limits() {
    let f = fixture().await;
    let zip = crate::tools::archive::tests::build_zip(&[("a.txt", b"hello")], None);
    seed(&f, "/uploads/pack.zip", &zip).await;
    limit_mb_remaining(&f, 1_000).await;

    let v = extract(&f, "/uploads/pack.zip", None, false, None).await.unwrap();

    assert_eq!(v["destination"], "/uploads/pack");
    assert_eq!(v["files_written"], 1);
    assert_eq!(v["dirs_created"], 1);
    assert_eq!(v["bytes_written"], 5);
    assert_eq!(read(&f, "/uploads/pack/a.txt").await.unwrap(), b"hello");
    assert_eq!(remaining_storage(&f).await, 995);
}

#[tokio::test]
async fn e2e_0085_003_net_growth_equal_to_the_remaining_st() {
    let f = fixture().await;
    let zip = crate::tools::archive::tests::build_zip(&[("a.txt", b"hello")], None);
    seed(&f, "/uploads/pack.zip", &zip).await;
    limit_mb_remaining(&f, 5).await;

    let v = extract(&f, "/uploads/pack.zip", None, false, None).await.unwrap();

    assert_eq!(v["bytes_written"], 5);
    assert_eq!(remaining_storage(&f).await, 0);
}

async fn assert_unchanged(f: &Fixture, remaining: i64, allowed: i64) {
    assert_eq!(held(), 0);
    assert_eq!(remaining_storage(f).await, remaining);
    assert_eq!(allowance(f), allowed);
}

#[tokio::test]
async fn e2e_0085_006_zip_bomb_against_the_storage_limit() {
    let f = fixture().await;
    let bomb = zip_bomb();
    assert!(bomb.len() <= 20_000, "{}", bomb.len());
    seed(&f, "/uploads/bomb.zip", &bomb).await;
    limit_mb_remaining(&f, 1_000).await;
    let before = allowance(&f);
    held_reset();

    let v = extract(&f, "/uploads/bomb.zip", None, false, None).await;

    project_refusal(&v);
    assert_eq!(
        v.unwrap_err().message,
        "project storage quota of 1 MB exceeded: 1047576 bytes used, 10000000 bytes requested"
    );
    assert_unchanged(&f, 1_000, before).await;
    assert!(!exists(&f, "/uploads/bomb").await);
    assert_eq!(extraction_audit_lines(&f, "/uploads/bomb.zip"), 0);
}

#[tokio::test]
async fn e2e_0085_007_7z_bomb_against_the_session_allowanc() {
    let f = fixture_allowance(1_000).await;
    seed(&f, "/uploads/bomb.7z", &sevenz_bomb()).await;
    held_reset();

    let v = extract(&f, "/uploads/bomb.7z", None, false, None).await;

    session_refusal(&v, 1_000);
    assert_eq!(held(), 0);
    assert!(!exists(&f, "/uploads/bomb").await);
    write(&f, "/uploads/small.bin", 1_000).await.expect("nothing was charged");
}

#[tokio::test]
async fn e2e_0085_009_tie_between_the_two_limits() {
    let f = fixture_allowance(1_000).await;
    seed(&f, "/uploads/bomb.zip", &zip_bomb()).await;
    limit_mb_remaining(&f, 1_000).await;

    let v = extract(&f, "/uploads/bomb.zip", None, false, None).await;

    project_refusal(&v);
    assert_eq!(remaining_storage(&f).await, 1_000);
    assert_eq!(allowance(&f), 1_000);
}

#[tokio::test]
async fn e2e_0085_010_the_session_allowance_is_lower() {
    let f = fixture_allowance(1_000).await;
    seed(&f, "/uploads/mid.zip", &mid_zip("a.bin", 1_500)).await;
    limit_remaining(&f, 2_000_000).await;
    held_reset();

    let v = extract(&f, "/uploads/mid.zip", None, false, None).await;

    session_refusal(&v, 1_000);
    assert_eq!(held(), 0);
    assert_eq!(remaining_storage(&f).await, 2_000_000);
}

#[tokio::test]
async fn e2e_0085_011_one_byte_over_the_remaining_storage() {
    let f = fixture().await;
    seed(&f, "/uploads/over.zip", &mid_zip("a.bin", 1_001)).await;
    limit_mb_remaining(&f, 1_000).await;
    held_reset();

    let v = extract(&f, "/uploads/over.zip", None, false, None).await;

    project_refusal(&v);
    assert_eq!(
        v.unwrap_err().message,
        "project storage quota of 1 MB exceeded: 1047576 bytes used, 1001 bytes requested"
    );
    assert_eq!(held(), 0);
    assert!(!exists(&f, "/uploads/over").await);
}

#[tokio::test]
async fn e2e_0085_012_entries_that_each_fit_but_not_togeth() {
    let f = fixture().await;
    let part = abc_bytes(400);
    let zip = crate::tools::archive::tests::build_zip(
        &[("a.bin", &part), ("b.bin", &part), ("c.bin", &part)],
        None,
    );
    seed(&f, "/uploads/three.zip", &zip).await;
    limit_mb_remaining(&f, 1_000).await;
    held_reset();

    let v = extract(&f, "/uploads/three.zip", None, false, None).await;

    project_refusal(&v);
    assert_eq!(
        v.unwrap_err().message,
        "project storage quota of 1 MB exceeded: 1047576 bytes used, 1200 bytes requested"
    );
    assert_eq!(held(), 0);
    assert!(!exists(&f, "/uploads/three").await);
}

#[tokio::test]
async fn e2e_0085_029_a_7z_link_entry_wins_over_the_limit() {
    let f = fixture().await;
    let zeros = vec![0u8; 10_000_000];
    let bytes = crate::tools::archive::tests::build_sevenz_ext(
        &[("zeros.bin", &zeros, None), ("link", b"/etc/passwd", Some(SEVENZ_LINK))],
        None,
        false,
    );
    seed(&f, "/uploads/bomb.7z", &bytes).await;
    limit_mb_remaining(&f, 1_000).await;
    held_reset();

    let v = extract(&f, "/uploads/bomb.7z", None, false, None).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::NOT_SUPPORTED, "{e:?}");
    assert_eq!(e.message, "archive entry 'link' is a symlink, which is not supported");
    assert_eq!(held(), 0);
}

#[tokio::test]
async fn e2e_0085_033_a_7z_escaping_entry_wins_over_earlie() {
    let f = fixture().await;
    let mut bytes = crate::tools::archive::tests::build_sevenz_ext(
        &[("a.txt", b"hello", None), ("../evil.txt", b"x", None)],
        None,
        false,
    );
    // The first block's packed bytes start right after the 32 byte signature header.
    for b in &mut bytes[32..38] {
        *b ^= 0xFF;
    }
    seed(&f, "/uploads/mix.7z", &bytes).await;

    let v = extract(&f, "/uploads/mix.7z", None, false, None).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS, "{e:?}");
    assert_eq!(e.message, "archive entry path escapes the destination: '../evil.txt'");
}

#[tokio::test]
async fn e2e_0085_034_a_7z_special_entry_wins_over_a_wrong() {
    let f = fixture().await;
    let bytes = crate::tools::archive::tests::build_sevenz_ext(
        &[("a.txt", b"hello", None), ("link", b"x", Some(SEVENZ_LINK))],
        Some("s3cret"),
        true,
    );
    seed(&f, "/uploads/vault.7z", &bytes).await;

    let v = extract(&f, "/uploads/vault.7z", None, false, Some("nope")).await;

    let e = v.expect_err("the extraction must be refused");
    assert_eq!(e.code, code::NOT_SUPPORTED, "{e:?}");
    assert_eq!(e.message, "archive entry 'link' is a symlink, which is not supported");
}

#[tokio::test]
async fn e2e_0085_039_a_repeated_refusal_stays_a_refusal_a() {
    let f = fixture().await;
    seed(&f, "/uploads/bomb.zip", &zip_bomb()).await;
    let pack = crate::tools::archive::tests::build_zip(&[("a.txt", b"hello")], None);
    seed(&f, "/uploads/pack.zip", &pack).await;
    limit_mb_remaining(&f, 1_000).await;
    let message =
        "project storage quota of 1 MB exceeded: 1047576 bytes used, 10000000 bytes requested";

    for _ in 0..2 {
        held_reset();
        let v = extract(&f, "/uploads/bomb.zip", None, false, None).await;
        project_refusal(&v);
        assert_eq!(v.unwrap_err().message, message);
        assert_eq!(held(), 0);
        assert_eq!(remaining_storage(&f).await, 1_000);
    }

    extract(&f, "/uploads/pack.zip", None, false, None).await.unwrap();
    assert_eq!(remaining_storage(&f).await, 995);
}

#[tokio::test]
async fn e2e_0085_041_solid_7z_bomb_counted_by_its_entries() {
    let f = fixture().await;
    let zeros = vec![0u8; 4_000_000];
    let bytes = sevenz_solid(&[("a.bin", &zeros), ("b.bin", &zeros), ("c.bin", &zeros)]);
    seed(&f, "/uploads/solid.7z", &bytes).await;
    limit_mb_remaining(&f, 1_000).await;
    held_reset();

    let v = extract(&f, "/uploads/solid.7z", None, false, None).await;

    project_refusal(&v);
    let e = v.unwrap_err();
    assert!(e.message.ends_with("12000000 bytes requested"), "{}", e.message);
    assert_eq!(held(), 0);
}

#[tokio::test]
async fn e2e_0085_sec_005_7z_size_of_2_63() {
    let f = fixture().await;
    seed(&f, "/uploads/free.7z", &sevenz_declaring("free.bin", TWO_63, b"hello")).await;
    limit_mb_remaining(&f, 1_000).await;

    let v = extract(&f, "/uploads/free.7z", None, false, None).await;

    project_refusal(&v);
    let e = v.unwrap_err();
    assert!(e.message.ends_with("9223372036854775807 bytes requested"), "{}", e.message);
    assert_eq!(held(), 0);
    assert_eq!(used(&f).await, 1_047_576);
    assert!(!exists(&f, "/uploads/free").await);
}
