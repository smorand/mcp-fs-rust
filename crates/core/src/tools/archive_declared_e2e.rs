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
