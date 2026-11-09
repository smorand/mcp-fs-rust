//! Helpers shared by the SPEC-0084 to SPEC-0087 archive tests (SPEC-0084 DEC-008).

use crate::errors::{Result, code};
use crate::tools::admin::test_support::Fixture;
use crate::tools::archive::extract_archive;
use crate::tools::archive_single::LAST_HIGH_WATER;
use serde_json::Value;
use std::io::Write as _;

pub(crate) const MOUNT: &str = "atlas";
pub(crate) const OWNER: &str = "mia@example.com";

/// A project with an empty `/uploads` folder.
pub(crate) async fn fixture() -> Fixture {
    let f = Fixture::with_config(|_| {}).await;
    f.seed_project(MOUNT, OWNER).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/uploads", true).await.unwrap();
    f
}

pub(crate) async fn seed(f: &Fixture, path: &str, content: &[u8]) {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.write_bytes_atomic(path, content).await.unwrap();
}

/// Authorizes then extracts, as both doors do.
pub(crate) async fn extract(
    f: &Fixture,
    path: &str,
    destination: Option<&str>,
    overwrite: bool,
    password: Option<&str>,
) -> Result<Value> {
    f.state.authorize(MOUNT, OWNER).await.unwrap();
    extract_archive(&f.state, MOUNT, OWNER, path, destination, overwrite, password).await
}

pub(crate) async fn read(f: &Fixture, path: &str) -> Option<Vec<u8>> {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    if c.exists(path).await.unwrap() { Some(c.read_bytes(path).await.unwrap()) } else { None }
}

pub(crate) async fn exists(f: &Fixture, path: &str) -> bool {
    f.state.stores.client(MOUNT).await.unwrap().exists(path).await.unwrap()
}

/// The refusal of a damaged archive of `format`.
pub(crate) fn corrupt_refusal(v: &Result<Value>, format: &str) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::INVALID_ARGUMENT, "{e:?}");
    let prefix = format!("archive is corrupt or not a valid {format} file:");
    assert!(e.message.starts_with(&prefix), "{}", e.message);
}

/// A project with an empty `/uploads` folder and a session write allowance of `n` bytes.
pub(crate) async fn fixture_allowance(n: i64) -> Fixture {
    let f = Fixture::with_config(move |c| c.safety.write_quota_bytes = n).await;
    f.seed_project(MOUNT, OWNER).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/uploads", true).await.unwrap();
    f
}

/// Give the project exactly `n` bytes of remaining storage.
pub(crate) async fn limit_remaining(f: &Fixture, n: i64) {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used = crate::quota::used_bytes(&c).await.unwrap();
    f.state.admin.set_quota(MOUNT, Some(used + n)).await.unwrap();
}

pub(crate) async fn remaining_storage(f: &Fixture) -> i64 {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let limit = f.state.safety.project_limit(MOUNT).await.unwrap().unwrap();
    limit - crate::quota::used_bytes(&c).await.unwrap()
}

pub(crate) fn allowance(f: &Fixture) -> i64 {
    f.state.safety.session_remaining(OWNER, MOUNT)
}

/// Forget the bytes the last decode held, so a test reads only its own call.
pub(crate) fn held_reset() {
    LAST_HIGH_WATER.with(|c| c.set(None));
}

/// Bytes the last decode held, 0 when the capped decoder never ran.
pub(crate) fn held() -> u64 {
    LAST_HIGH_WATER.with(std::cell::Cell::get).unwrap_or(0)
}

/// Bytes the last decode held; fails when the capped decoder never ran.
pub(crate) fn held_some() -> u64 {
    LAST_HIGH_WATER.with(std::cell::Cell::get).expect("no capped decode ran")
}

/// The decode held at most `max` bytes (the tokio test runtime is single
/// threaded, so the thread local belongs to this test alone).
pub(crate) fn held_at_most(max: u64) {
    let held = held_some();
    assert!(held <= max, "the extraction held {held} bytes, more than {max}");
}

/// The audit lines the extractions of `path` left in the session.
pub(crate) fn extraction_audit_lines(f: &Fixture, path: &str) -> usize {
    let log = f.state.safety.audit(OWNER, MOUNT);
    log.iter().filter(|e| e.op == "extract_archive" && e.path == path).count()
}

pub(crate) fn project_refusal(v: &Result<Value>) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::PROJECT_QUOTA_EXCEEDED, "{e:?}");
    assert!(e.message.starts_with("project storage quota of"), "{}", e.message);
}

pub(crate) fn session_refusal(v: &Result<Value>, quota: i64) {
    let e = v.as_ref().expect_err("the extraction must be refused");
    assert_eq!(e.code, code::WRITE_QUOTA_EXCEEDED, "{e:?}");
    assert_eq!(e.message, format!("session write quota of {quota} bytes exceeded"));
}

/// A tar of `entries` (path, content), regular files only.
pub(crate) fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut b = tar::Builder::new(&mut out);
        for (path, content) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_path(path).unwrap();
            h.set_size(content.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append(&h, *content).unwrap();
        }
        b.finish().unwrap();
    }
    out
}

pub(crate) fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

pub(crate) fn bz2(bytes: &[u8]) -> Vec<u8> {
    let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

pub(crate) fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut e = liblzma::write::XzEncoder::new(Vec::new(), 6);
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

/// A tar of `zeros.bin`, 10 000 000 zero bytes: a small file once compressed.
pub(crate) fn tar_bomb() -> Vec<u8> {
    tar_bytes(&[("zeros.bin", &vec![0u8; 10_000_000])])
}

/// A tar whose first entry is the symbolic link `link_name` to `target`, then `then` as regular files.
pub(crate) fn tar_with_symlink(link_name: &str, target: &str, then: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut b = tar::Builder::new(&mut out);
        let mut h = tar::Header::new_gnu();
        h.set_path(link_name).unwrap();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_link_name(target).unwrap();
        h.set_cksum();
        b.append(&h, std::io::empty()).unwrap();
        for (path, content) in then {
            let mut h = tar::Header::new_gnu();
            h.set_path(path).unwrap();
            h.set_size(content.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append(&h, *content).unwrap();
        }
        b.finish().unwrap();
    }
    out
}

/// One GNU header declaring `declared` bytes, then `content` and nothing after: the entry lies.
pub(crate) fn tar_lying(name: &str, declared: u64, content: &[u8]) -> Vec<u8> {
    let mut h = tar::Header::new_gnu();
    h.set_path(name).unwrap();
    h.set_size(declared);
    h.set_mode(0o644);
    h.set_cksum();
    let mut out = h.as_bytes().to_vec();
    out.extend_from_slice(content);
    out
}

/// `abc` repeated up to `n` bytes.
pub(crate) fn abc_bytes(n: usize) -> Vec<u8> {
    b"abc".iter().copied().cycle().take(n).collect()
}

/// The second member of `atlas`, who reads what the extraction left behind.
pub(crate) const SECOND: &str = "noa@example.com";

/// Add [`SECOND`] to `atlas`.
pub(crate) async fn add_second_member(f: &Fixture) -> &'static str {
    f.state.admin.add_member(MOUNT, SECOND, OWNER).await.unwrap();
    SECOND
}

/// A 1 048 576 byte limit with exactly `n` bytes left: `/filler.bin` takes
/// what the files already stored (the archive included) leave over.
pub(crate) async fn limit_mb_remaining(f: &Fixture, n: i64) {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used = crate::quota::used_bytes(&c).await.unwrap();
    let filler = 1_048_576 - n - used;
    assert!(filler >= 0, "the stored files already exceed 1 MB minus {n}");
    c.write_bytes_atomic("/filler.bin", &vec![b'f'; filler as usize]).await.unwrap();
    f.state.admin.set_quota(MOUNT, Some(1_048_576)).await.unwrap();
}

/// Like [`limit_mb_remaining`] when the stored files exceed 1 MB: the limit is
/// the next whole megabyte above them, so the figures in the message are not
/// fixed and a test asserts only its prefix or its tail.
pub(crate) async fn limit_whole_mb_remaining(f: &Fixture, n: i64) {
    let c = f.state.stores.client(MOUNT).await.unwrap();
    let used = crate::quota::used_bytes(&c).await.unwrap();
    let mb = 1_048_576;
    let limit = (used + n + mb - 1) / mb * mb;
    c.write_bytes_atomic("/filler.bin", &vec![b'f'; (limit - n - used) as usize]).await.unwrap();
    f.state.admin.set_quota(MOUNT, Some(limit)).await.unwrap();
}

/// A zip whose entries (name, declared size, content) declare any size the
/// format can carry: no library writes a lying header (SPEC-0085 DEC-009).
/// The content is deflated with a checksum of the real bytes; a declared
/// size of `0xFFFF_FFFF` or more puts `0xFFFF_FFFF` in both size fields of
/// both headers plus a zip64 extra field (uncompressed then compressed size).
pub(crate) fn zip_declaring(entries: &[(&str, u64, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, declared, content) in entries {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(content).unwrap();
        let data = e.finish().unwrap();
        let mut crc = flate2::Crc::new();
        crc.update(content);
        let zip64 = *declared >= 0xFFFF_FFFF;
        let (version, usize32, csize32, extra) = if zip64 {
            let mut x = Vec::new();
            x.extend_from_slice(&1u16.to_le_bytes());
            x.extend_from_slice(&16u16.to_le_bytes());
            x.extend_from_slice(&declared.to_le_bytes());
            x.extend_from_slice(&(data.len() as u64).to_le_bytes());
            (45u16, 0xFFFF_FFFFu32, 0xFFFF_FFFFu32, x)
        } else {
            (20u16, *declared as u32, data.len() as u32, Vec::new())
        };
        let offset = out.len() as u32;
        // Shared fields: version needed, flags, deflate, time, date, crc, sizes, name and extra lengths.
        let mut common = Vec::new();
        common.extend_from_slice(&version.to_le_bytes());
        common.extend_from_slice(&0u16.to_le_bytes());
        common.extend_from_slice(&8u16.to_le_bytes());
        common.extend_from_slice(&0u16.to_le_bytes());
        common.extend_from_slice(&0x21u16.to_le_bytes());
        common.extend_from_slice(&crc.sum().to_le_bytes());
        common.extend_from_slice(&csize32.to_le_bytes());
        common.extend_from_slice(&usize32.to_le_bytes());
        common.extend_from_slice(&(name.len() as u16).to_le_bytes());
        common.extend_from_slice(&(extra.len() as u16).to_le_bytes());

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&extra);
        out.extend_from_slice(&data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&45u16.to_le_bytes());
        central.extend_from_slice(&common);
        central.extend_from_slice(&0u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&extra);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// A zip of `zeros.bin`, 10 000 000 zero bytes: about 10 000 bytes once deflated.
pub(crate) fn zip_bomb() -> Vec<u8> {
    crate::tools::archive::tests::build_zip(&[("zeros.bin", &vec![0u8; 10_000_000])], None)
}

/// A 7z of `zeros.bin`, 10 000 000 zero bytes: a few hundred bytes once compressed.
pub(crate) fn sevenz_bomb() -> Vec<u8> {
    crate::tools::archive::tests::build_sevenz_ext(
        &[("zeros.bin", &vec![0u8; 10_000_000], None)],
        None,
        false,
    )
}

/// A 7z holding `entries` (name, content) in ONE compressed block (solid).
pub(crate) fn sevenz_solid(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};
    let mut buf = Vec::new();
    {
        let mut writer = ArchiveWriter::new(std::io::Cursor::new(&mut buf)).unwrap();
        let metas = entries.iter().map(|(name, _)| ArchiveEntry::new_file(name)).collect();
        let readers = entries
            .iter()
            .map(|(_, content)| SourceReader::from(std::io::Cursor::new(*content)))
            .collect();
        writer.push_archive_entries(metas, readers).unwrap();
        writer.finish().unwrap();
    }
    buf
}

/// A 7z number: one byte below 0x80, else 0xFF and eight little endian bytes.
fn sevenz_number(n: u64) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let mut out = vec![0xFF];
    out.extend_from_slice(&n.to_le_bytes());
    out
}

/// A 7z with one entry `name` stored with the Copy coder, whose unpack size
/// says `declared` while `data` is all it holds: no library writes a lying
/// header (SPEC-0085 DEC-009). No CRC digest, plain header, valid start header CRC.
pub(crate) fn sevenz_declaring(name: &str, declared: u64, data: &[u8]) -> Vec<u8> {
    let crc = |bytes: &[u8]| {
        let mut c = flate2::Crc::new();
        c.update(bytes);
        c.sum()
    };
    let mut header = vec![0x01, 0x04];
    // PackInfo: pack position 0, one pack stream of data.len() bytes.
    header.extend_from_slice(&[0x06, 0x00, 0x01, 0x09]);
    header.extend(sevenz_number(data.len() as u64));
    header.push(0x00);
    // UnpackInfo: one folder, one simple Copy coder, the declared unpack size.
    header.extend_from_slice(&[0x07, 0x0B, 0x01, 0x00, 0x01, 0x01, 0x00, 0x0C]);
    header.extend(sevenz_number(declared));
    header.push(0x00);
    // SubStreamsInfo with nothing in it: one stream per folder, no digest. The
    // reader refuses a file with data and no such section.
    header.extend_from_slice(&[0x08, 0x00]);
    header.push(0x00); // end of the streams info
    // FilesInfo: one file, its UTF-16 name.
    let mut names = vec![0x00];
    for unit in name.encode_utf16().chain(std::iter::once(0)) {
        names.extend_from_slice(&unit.to_le_bytes());
    }
    header.extend_from_slice(&[0x05, 0x01, 0x11]);
    header.extend(sevenz_number(names.len() as u64));
    header.extend_from_slice(&names);
    header.push(0x00);
    header.push(0x00); // end of the header

    let mut next = Vec::new();
    next.extend_from_slice(&(data.len() as u64).to_le_bytes());
    next.extend_from_slice(&(header.len() as u64).to_le_bytes());
    next.extend_from_slice(&crc(&header).to_le_bytes());
    let mut out = vec![b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04];
    out.extend_from_slice(&crc(&next).to_le_bytes());
    out.extend_from_slice(&next);
    out.extend_from_slice(data);
    out.extend_from_slice(&header);
    out
}
