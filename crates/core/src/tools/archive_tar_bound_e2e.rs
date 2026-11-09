//! Tar family decoding (SPEC-0084): `.tar.xz` and `.txz` go through the bare xz decoder.

use super::archive_testkit::{
    abc_bytes, allowance, bz2, corrupt_refusal, exists, extract, extraction_audit_lines, fixture,
    fixture_allowance, gz, held_at_most, held_reset, limit_remaining, project_refusal, read, seed,
    session_refusal, tar_bomb, tar_bytes, tar_lying, tar_with_symlink, xz,
};
use liblzma::stream::{Check, Filters, LzmaOptions, Stream};
use serde_json::json;
use std::io::Write as _;

/// A tar holding `a.txt` = `hello`: 2048 bytes (header, one data block, two end blocks).
fn tar() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut b = tar::Builder::new(&mut out);
        let mut h = tar::Header::new_gnu();
        h.set_path("a.txt").unwrap();
        h.set_size(5);
        h.set_cksum();
        b.append(&h, &b"hello"[..]).unwrap();
        b.finish().unwrap();
    }
    out
}

fn xz_with(content: &[u8], filters: &Filters, check: Check) -> Vec<u8> {
    let stream = Stream::new_stream_encoder(filters, check).unwrap();
    let mut enc = liblzma::write::XzEncoder::new_stream(Vec::new(), stream);
    enc.write_all(content).unwrap();
    enc.finish().unwrap()
}

fn lzma2() -> Filters {
    let mut filters = Filters::new();
    filters.lzma2(&LzmaOptions::new_preset(6).unwrap());
    filters
}

/// The tar cut after its first 512 bytes, as two xz streams.
fn two_streams(check: Check) -> Vec<u8> {
    let t = tar();
    let mut out = xz_with(&t[..512], &lzma2(), check);
    out.extend(xz_with(&t[512..], &lzma2(), check));
    out
}

async fn assert_hello(f: &crate::tools::admin::test_support::Fixture) {
    assert_eq!(read(f, "/uploads/pack/a.txt").await.as_deref(), Some(&b"hello"[..]));
}

async fn refused_corrupt(bytes: &[u8]) {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.xz", bytes).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    corrupt_refusal(&v, "tar.xz");
    assert!(!exists(&f, "/uploads/pack").await);
}

#[tokio::test]
async fn e2e_0084_026_two_xz_streams() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.xz", &two_streams(Check::Crc64)).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    assert_eq!(
        v.as_ref().ok(),
        Some(&json!({"destination": "/uploads/pack", "files_written": 1,
                     "dirs_created": 1, "bytes_written": 5})),
        "{v:?}"
    );
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_027_x86_branch_filter() {
    let f = fixture().await;
    let mut filters = Filters::new();
    filters.x86();
    filters.lzma2(&LzmaOptions::new_preset(6).unwrap());
    seed(&f, "/uploads/pack.tar.xz", &xz_with(&tar(), &filters, Check::Crc64)).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_028_sha_256_integrity_check() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.xz", &xz_with(&tar(), &lzma2(), Check::Sha256)).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_029_txz_of_two_streams_with_a_crc32_chec() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.txz", &two_streams(Check::Crc32)).await;
    let v = extract(&f, "/uploads/pack.txz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_030_4_zero_bytes_after_the_stream() {
    let f = fixture().await;
    let mut bytes = xz(&tar());
    bytes.extend([0u8; 4]);
    seed(&f, "/uploads/pack.tar.xz", &bytes).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_031_4_zero_bytes_between_two_streams() {
    let f = fixture().await;
    let t = tar();
    let mut bytes = xz(&t[..512]);
    bytes.extend([0u8; 4]);
    bytes.extend(xz(&t[512..]));
    seed(&f, "/uploads/pack.tar.xz", &bytes).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_032_empty_stream_between_two_streams() {
    let f = fixture().await;
    let t = tar();
    let mut bytes = xz(&t[..512]);
    bytes.extend(xz(b""));
    bytes.extend(xz(&t[512..]));
    seed(&f, "/uploads/pack.tar.xz", &bytes).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_hello(&f).await;
}

#[tokio::test]
async fn e2e_0084_033_3_zero_bytes_after_the_stream() {
    let mut bytes = xz(&tar());
    bytes.extend([0u8; 3]);
    refused_corrupt(&bytes).await;
}

#[tokio::test]
async fn e2e_0084_034_other_bytes_after_the_stream() {
    let mut bytes = xz(&tar());
    bytes.extend(b"JUNK");
    refused_corrupt(&bytes).await;
}

#[tokio::test]
async fn e2e_0084_035_legacy_lzma_file_named_tar_xz() {
    let stream = Stream::new_lzma_encoder(&LzmaOptions::new_preset(6).unwrap()).unwrap();
    let mut enc = liblzma::write::XzEncoder::new_stream(Vec::new(), stream);
    enc.write_all(&tar()).unwrap();
    refused_corrupt(&enc.finish().unwrap()).await;
}

#[tokio::test]
async fn e2e_0084_036_integrity_check_type_xz_does_not_def() {
    let mut file = xz_with(&tar(), &lzma2(), Check::Crc32);
    let crc = |bytes: &[u8]| {
        let mut c = flate2::Crc::new();
        c.update(bytes);
        c.sum().to_le_bytes()
    };
    let len = file.len();
    file[7] = 0x03;
    file[len - 3] = 0x03;
    let header_crc = crc(&file[6..8]);
    file[8..12].copy_from_slice(&header_crc);
    let footer_crc = crc(&file[len - 8..len - 2]);
    file[len - 12..len - 8].copy_from_slice(&footer_crc);
    refused_corrupt(&file).await;
}

#[tokio::test]
async fn e2e_0084_037_truncated_and_empty_files() {
    let f = fixture().await;
    let full = xz(&tar());
    seed(&f, "/uploads/cut.tar.xz", &full[..full.len() / 2]).await;
    seed(&f, "/uploads/zero.tar.xz", b"").await;
    for (path, dir) in
        [("/uploads/cut.tar.xz", "/uploads/cut"), ("/uploads/zero.tar.xz", "/uploads/zero")]
    {
        let v = extract(&f, path, None, false, None).await;
        corrupt_refusal(&v, "tar.xz");
        assert!(!exists(&f, dir).await, "{dir}");
    }
}

#[tokio::test]
async fn e2e_0084_038_damaged_compressed_byte() {
    let mut bytes = xz(&tar());
    // Inside the LZMA2 payload, past the 12 byte stream header and the block header.
    bytes[30] ^= 0xFF;
    refused_corrupt(&bytes).await;
}

// Decoding bound (US-0184).

/// The decompressed archive length at which the bound sits for 1 000 bytes of room.
const FIT: usize = 1_049_576;

/// `tar()` followed by zero bytes up to `len` bytes.
fn tar_padded(len: usize) -> Vec<u8> {
    let mut t = tar();
    t.resize(len, 0);
    t
}

async fn remaining(f: &crate::tools::admin::test_support::Fixture) -> i64 {
    super::archive_testkit::remaining_storage(f).await
}

#[tokio::test]
async fn e2e_0084_004_files_equal_to_the_remaining_storage() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.gz", &gz(&tar())).await;
    limit_remaining(&f, 5).await;
    let v = extract(&f, "/uploads/pack.tar.gz", None, false, None).await;
    assert_eq!(
        v.as_ref().ok(),
        Some(&json!({"destination": "/uploads/pack", "files_written": 1,
                     "dirs_created": 1, "bytes_written": 5})),
        "{v:?}"
    );
    assert_eq!(remaining(&f).await, 0);
}

#[tokio::test]
async fn e2e_0084_005_files_over_the_remaining_storage_arc() {
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.gz", &gz(&tar())).await;
    limit_remaining(&f, 4).await;
    let v = extract(&f, "/uploads/pack.tar.gz", None, false, None).await;
    project_refusal(&v);
    assert!(!exists(&f, "/uploads/pack").await);
    assert_eq!(remaining(&f).await, 4);
}

#[tokio::test]
async fn e2e_0084_006_files_over_the_session_allowance_arc() {
    let f = fixture_allowance(4).await;
    seed(&f, "/uploads/pack.tar.xz", &xz(&tar())).await;
    let v = extract(&f, "/uploads/pack.tar.xz", None, false, None).await;
    session_refusal(&v, 4);
    assert!(!exists(&f, "/uploads/pack").await);
}

/// Seed a bomb under `path` and check its size stays a small file.
async fn seed_bomb(f: &crate::tools::admin::test_support::Fixture, path: &str, bytes: Vec<u8>) {
    assert!(bytes.len() <= 20_000, "the bomb is {} bytes", bytes.len());
    seed(f, path, &bytes).await;
}

#[tokio::test]
async fn e2e_0084_008_tar_gz_bomb_against_the_storage_limi() {
    let f = fixture().await;
    seed_bomb(&f, "/uploads/bomb.tar.gz", gz(&tar_bomb())).await;
    limit_remaining(&f, 1000).await;
    held_reset();
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    project_refusal(&v);
    held_at_most(1_049_576);
    assert!(!exists(&f, "/uploads/bomb").await);
    assert_eq!(remaining(&f).await, 1000);
}

#[tokio::test]
async fn e2e_0084_009_tar_bz2_bomb_against_the_session_all() {
    let f = fixture_allowance(1000).await;
    seed_bomb(&f, "/uploads/bomb.tar.bz2", bz2(&tar_bomb())).await;
    held_reset();
    let v = extract(&f, "/uploads/bomb.tar.bz2", None, false, None).await;
    session_refusal(&v, 1000);
    held_at_most(1_049_576);
    assert!(!exists(&f, "/uploads/bomb").await);
    let c = f.state.stores.client(super::archive_testkit::MOUNT).await.unwrap();
    let w = crate::core::fs_ops::write_bytes(
        &c,
        &f.state.safety,
        super::archive_testkit::OWNER,
        super::archive_testkit::MOUNT,
        "/uploads/small.bin",
        &[7u8; 1000],
        false,
        true,
    )
    .await;
    assert!(w.is_ok(), "{w:?}");
}

#[tokio::test]
async fn e2e_0084_010_tar_xz_bomb_against_the_storage_limi() {
    let f = fixture().await;
    seed_bomb(&f, "/uploads/bomb.tar.xz", xz(&tar_bomb())).await;
    limit_remaining(&f, 1000).await;
    held_reset();
    let v = extract(&f, "/uploads/bomb.tar.xz", None, false, None).await;
    project_refusal(&v);
    held_at_most(1_049_576);
    assert!(!exists(&f, "/uploads/bomb").await);
    assert_eq!(remaining(&f).await, 1000);
}

#[tokio::test]
async fn e2e_0084_011_txz_bomb_against_the_session_allowan() {
    let f = fixture_allowance(1000).await;
    seed_bomb(&f, "/uploads/BOMB.TXZ", xz(&tar_bomb())).await;
    held_reset();
    let v = extract(&f, "/uploads/BOMB.TXZ", None, false, None).await;
    session_refusal(&v, 1000);
    held_at_most(1_049_576);
    assert!(!exists(&f, "/uploads/BOMB").await);
}

#[tokio::test]
async fn e2e_0084_012_tie_between_the_two_sides() {
    let f = fixture_allowance(1000).await;
    seed_bomb(&f, "/uploads/bomb.tar.gz", gz(&tar_bomb())).await;
    limit_remaining(&f, 1000).await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    project_refusal(&v);
    assert_eq!(remaining(&f).await, 1000);
    assert_eq!(allowance(&f), 1000);
}

#[tokio::test]
async fn e2e_0084_013_the_session_side_is_lower() {
    let f = fixture_allowance(1000).await;
    seed_bomb(&f, "/uploads/bomb.tar.gz", gz(&tar_bomb())).await;
    limit_remaining(&f, 2_000_000).await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    session_refusal(&v, 1000);
    assert_eq!(remaining(&f).await, 2_000_000);
}

#[tokio::test]
async fn e2e_0084_014_decompressed_archive_exactly_at_the() {
    let f = fixture().await;
    seed(&f, "/uploads/fit.tar.gz", &gz(&tar_padded(FIT))).await;
    limit_remaining(&f, 1000).await;
    let v = extract(&f, "/uploads/fit.tar.gz", None, false, None).await;
    assert_eq!(
        v.as_ref().ok(),
        Some(&json!({"destination": "/uploads/fit", "files_written": 1,
                     "dirs_created": 1, "bytes_written": 5})),
        "{v:?}"
    );
    assert_eq!(remaining(&f).await, 995);
}

#[tokio::test]
async fn e2e_0084_015_one_byte_over_the_bound() {
    let f = fixture().await;
    seed(&f, "/uploads/fit.tar.gz", &gz(&tar_padded(FIT + 1))).await;
    limit_remaining(&f, 1000).await;
    let v = extract(&f, "/uploads/fit.tar.gz", None, false, None).await;
    project_refusal(&v);
    assert!(!exists(&f, "/uploads/fit").await);
    assert_eq!(remaining(&f).await, 1000);
}

#[tokio::test]
async fn e2e_0084_021_a_refusal_at_the_bound_is_not_audite() {
    let f = fixture().await;
    seed_bomb(&f, "/uploads/bomb.tar.gz", gz(&tar_bomb())).await;
    seed(&f, "/uploads/pack.tar.gz", &gz(&tar())).await;
    limit_remaining(&f, 1000).await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    project_refusal(&v);
    assert_eq!(extraction_audit_lines(&f, "/uploads/bomb.tar.gz"), 0);
    let v = extract(&f, "/uploads/pack.tar.gz", None, false, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_eq!(remaining(&f).await, 995);
}

#[tokio::test]
async fn e2e_0084_046_bytes_after_the_end_marker_count() {
    let f = fixture().await;
    seed_bomb(&f, "/uploads/tail.tar.gz", gz(&tar_padded(10_000_000))).await;
    limit_remaining(&f, 1000).await;
    held_reset();
    let v = extract(&f, "/uploads/tail.tar.gz", None, false, None).await;
    project_refusal(&v);
    held_at_most(1_049_576);
    assert!(!exists(&f, "/uploads/tail").await);
}

// Order of the refusals, re extraction near the limit, lying entries (US-0185).

async fn bomb_fixture(name: &str, bytes: Vec<u8>) -> crate::tools::admin::test_support::Fixture {
    let f = fixture().await;
    seed_bomb(&f, name, bytes).await;
    limit_remaining(&f, 1000).await;
    f
}

#[tokio::test]
async fn e2e_0084_016_the_bound_wins_over_a_collision() {
    let f = bomb_fixture("/uploads/bomb.tar.gz", gz(&tar_bomb())).await;
    seed(&f, "/uploads/bomb/zeros.bin", b"old").await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    project_refusal(&v);
    assert_eq!(read(&f, "/uploads/bomb/zeros.bin").await.as_deref(), Some(&b"old"[..]));
}

#[tokio::test]
async fn e2e_0084_017_the_bound_wins_over_an_unsupported_e() {
    let t = tar_with_symlink("link", "/etc/passwd", &[("zeros.bin", &vec![0u8; 10_000_000])]);
    let f = bomb_fixture("/uploads/bomb.tar.gz", gz(&t)).await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    project_refusal(&v);
    assert!(!exists(&f, "/uploads/bomb").await);
}

#[tokio::test]
async fn e2e_0084_018_password_wins_over_the_bound() {
    let f = bomb_fixture("/uploads/bomb.tar.gz", gz(&tar_bomb())).await;
    held_reset();
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, Some("s3cret")).await;
    let e = v.as_ref().expect_err("refused");
    assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "{e:?}");
    assert_eq!(e.message, "password is not applicable to this archive format");
    assert_eq!(super::archive_testkit::held(), 0);
}

#[tokio::test]
async fn e2e_0084_019_corruption_met_first_wins_over_the_b() {
    let mut bytes = gz(&tar_bomb());
    bytes[0] = 0;
    bytes[1] = 0;
    let f = bomb_fixture("/uploads/bomb.tar.gz", bytes).await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    corrupt_refusal(&v, "tar.gz");
}

#[tokio::test]
async fn e2e_0084_020_the_bound_met_first_wins_over_a_late() {
    let mut bytes = gz(&tar_bomb());
    let n = bytes.len();
    for b in &mut bytes[n - 8..] {
        *b = !*b;
    }
    let f = bomb_fixture("/uploads/bomb.tar.gz", bytes).await;
    let v = extract(&f, "/uploads/bomb.tar.gz", None, false, None).await;
    project_refusal(&v);
}

/// `pack.tar.gz` holding `big.bin` (3 000 000 bytes) with 100 000 bytes of room left once
/// `stored_at` (if any) holds the same bytes.
async fn pack_fixture(stored_at: &str) -> crate::tools::admin::test_support::Fixture {
    let big = abc_bytes(3_000_000);
    let f = fixture().await;
    seed(&f, "/uploads/pack.tar.gz", &gz(&tar_bytes(&[("big.bin", &big)]))).await;
    seed(&f, stored_at, &big).await;
    limit_remaining(&f, 100_000).await;
    f
}

#[tokio::test]
async fn e2e_0084_022_re_extract_with_overwrite_near_the_l() {
    let f = pack_fixture("/uploads/pack/big.bin").await;
    let v = extract(&f, "/uploads/pack.tar.gz", None, true, None).await;
    assert_eq!(
        v.as_ref().ok(),
        Some(&json!({"destination": "/uploads/pack", "files_written": 1,
                     "dirs_created": 0, "bytes_written": 3_000_000})),
        "{v:?}"
    );
    assert_eq!(remaining(&f).await, 100_000);
}

#[tokio::test]
async fn e2e_0084_023_same_situation_without_overwrite() {
    let f = pack_fixture("/uploads/pack/big.bin").await;
    let v = extract(&f, "/uploads/pack.tar.gz", None, false, None).await;
    let e = v.as_ref().expect_err("refused");
    assert_eq!(e.code, crate::errors::code::NO_CLOBBER, "{e:?}");
    assert_eq!(
        e.message,
        "'/uploads/pack/big.bin' already exists; pass overwrite=true to replace it"
    );
}

#[tokio::test]
async fn e2e_0084_024_files_stored_elsewhere_do_not_count() {
    let f = pack_fixture("/elsewhere/big.bin").await;
    held_reset();
    let v = extract(&f, "/uploads/pack.tar.gz", None, true, None).await;
    project_refusal(&v);
    held_at_most(1_148_576);
    assert!(!exists(&f, "/uploads/pack").await);
}

#[tokio::test]
async fn e2e_0084_025_explicit_destination_holding_the_fil() {
    let f = pack_fixture("/out/big.bin").await;
    let v = extract(&f, "/uploads/pack.tar.gz", Some("/out"), true, None).await;
    assert!(v.is_ok(), "{v:?}");
    assert_eq!(read(&f, "/out/big.bin").await, Some(abc_bytes(3_000_000)));
    assert_eq!(remaining(&f).await, 100_000);
}

async fn lying(name: &str, format: &str, bytes: Vec<u8>) {
    let f = fixture().await;
    seed(&f, name, &bytes).await;
    limit_remaining(&f, 1000).await;
    let v = extract(&f, name, None, false, None).await;
    corrupt_refusal(&v, format);
    assert!(!exists(&f, "/uploads/lie").await);
    assert_eq!(remaining(&f).await, 1000);
}

#[tokio::test]
async fn e2e_0084_043_tar_gz_entry_declaring_more_than_it() {
    lying("/uploads/lie.tar.gz", "tar.gz", gz(&tar_lying("a.txt", 1_000_000_000, b"hello"))).await;
}

#[tokio::test]
async fn e2e_0084_044_tar_bz2_entry_declaring_more_than_it() {
    lying("/uploads/lie.tar.bz2", "tar.bz2", bz2(&tar_lying("a.txt", 1_000_000_000, b"hello")))
        .await;
}

#[tokio::test]
async fn e2e_0084_045_tar_xz_entry_declaring_more_than_it() {
    lying("/uploads/lie.tar.xz", "tar.xz", xz(&tar_lying("a.txt", 1_000_000_000, b"hello"))).await;
}
