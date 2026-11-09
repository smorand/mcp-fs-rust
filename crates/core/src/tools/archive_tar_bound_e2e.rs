//! Tar family decoding (SPEC-0084): `.tar.xz` and `.txz` go through the bare xz decoder.

use super::archive_testkit::{corrupt_refusal, exists, extract, fixture, read, seed};
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

fn xz(content: &[u8]) -> Vec<u8> {
    xz_with(content, &lzma2(), Check::Crc64)
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
