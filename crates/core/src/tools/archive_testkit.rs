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
    let parts: Vec<RawPart> = entries
        .iter()
        .map(|(name, declared, content)| RawPart { declared: *declared, ..raw_part(name, content) })
        .collect();
    zip_raw(&parts)
}

/// One zip entry as written: deflated `data`, its checksum and the size it declares.
pub(crate) struct RawPart {
    pub(crate) name: String,
    pub(crate) declared: u64,
    pub(crate) crc: u32,
    pub(crate) data: Vec<u8>,
}

/// `content` deflated, declaring its real size.
pub(crate) fn raw_part(name: &str, content: &[u8]) -> RawPart {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(content).unwrap();
    let mut crc = flate2::Crc::new();
    crc.update(content);
    RawPart {
        name: name.to_string(),
        declared: content.len() as u64,
        crc: crc.sum(),
        data: e.finish().unwrap(),
    }
}

/// A zip of `entries` written byte for byte as given (SPEC-0088 DEC-011), so a
/// header can declare any size and carry any compressed stream.
pub(crate) fn zip_raw(entries: &[RawPart]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for RawPart { name, declared, crc, data } in entries {
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
        common.extend_from_slice(&crc.to_le_bytes());
        common.extend_from_slice(&csize32.to_le_bytes());
        common.extend_from_slice(&usize32.to_le_bytes());
        common.extend_from_slice(&(name.len() as u16).to_le_bytes());
        common.extend_from_slice(&(extra.len() as u16).to_le_bytes());

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&extra);
        out.extend_from_slice(data);

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

/// Letters per cached deflate segment of [`deflate_run`].
const SEGMENT: usize = 1_000_000;

/// Raw deflate of `data`. A full flush ends the output on a byte boundary with
/// no reference to anything before it, so independently compressed pieces
/// concatenate into one valid stream; `finish` closes it with the final block.
fn deflate_piece(data: &[u8], finish: bool) -> Vec<u8> {
    use flate2::{Compress, Compression, FlushCompress, Status};
    let mut c = Compress::new(Compression::default(), false);
    let mut out = Vec::with_capacity(data.len() / 4 + 4_096);
    let flush = if finish { FlushCompress::Finish } else { FlushCompress::Full };
    loop {
        if out.capacity() - out.len() < 4_096 {
            out.reserve(out.capacity());
        }
        let consumed = c.total_in() as usize;
        let status = c.compress_vec(&data[consumed..], &mut out, flush).unwrap();
        let all_in = c.total_in() as usize == data.len();
        let done =
            if finish { status == Status::StreamEnd } else { all_in && out.len() < out.capacity() };
        if done {
            return out;
        }
    }
}

/// The deflated segment of [`SEGMENT`] copies of `letter`, compressed once per
/// process and letter, and its checksum (`Crc` cannot be cloned, so it is
/// recomputed: one pass over 1 000 000 bytes).
fn segment(letter: u8) -> (Vec<u8>, flate2::Crc) {
    static CACHE: std::sync::Mutex<Vec<(u8, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());
    let letters = vec![letter; SEGMENT];
    let mut crc = flate2::Crc::new();
    crc.update(&letters);
    let mut cache = CACHE.lock().unwrap();
    if let Some((_, data)) = cache.iter().find(|(l, _)| *l == letter) {
        return (data.clone(), crc);
    }
    let data = deflate_piece(&letters, false);
    cache.push((letter, data.clone()));
    (data, crc)
}

/// The deflate stream of `prefix`, then `n` copies of `letter`, then `suffix`,
/// and its checksum: whole cached segments repeated, so a 200 000 000 byte part
/// costs about 200 KB and no 200 MB compression (SPEC-0088 DEC-011).
pub(crate) fn deflate_run(
    prefix: &str,
    letter: u8,
    n: usize,
    suffix: &str,
) -> (Vec<u8>, flate2::Crc) {
    let mut out = deflate_piece(prefix.as_bytes(), false);
    let mut crc = flate2::Crc::new();
    crc.update(prefix.as_bytes());
    let (seg, seg_crc) = segment(letter);
    for _ in 0..n / SEGMENT {
        out.extend_from_slice(&seg);
        crc.combine(&seg_crc);
    }
    let rest = vec![letter; n % SEGMENT];
    out.extend(deflate_piece(&rest, false));
    crc.update(&rest);
    out.extend(deflate_piece(suffix.as_bytes(), true));
    crc.update(suffix.as_bytes());
    (out, crc)
}

/// Size of a bomb part, markup included (SPEC-0088 section 9).
pub(crate) const BOMB: u64 = 200_000_000;

/// The markup around the letters of a bomb part named `name`: the letters are
/// the text where the part has text, else the value of a root attribute `x`.
fn bomb_markup(name: &str) -> (String, String) {
    let text = |open: &str, close: &str| (open.to_string(), close.to_string());
    let attribute = |root: &str| (format!("<{root} x=\""), "\"/>".to_string());
    if name == "word/document.xml" {
        text(
            r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>"#,
            "</w:t></w:r></w:p></w:body></w:document>",
        )
    } else if name.starts_with("ppt/slides/slide") || name.starts_with("ppt/notesSlides/") {
        text(r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>"#, "</a:t></a:r></a:p></p:sld>")
    } else if name == "xl/sharedStrings.xml" {
        text("<sst><si><t>", "</t></si></sst>")
    } else if name.ends_with(".rels") {
        attribute("Relationships")
    } else if name == "ppt/presentation.xml" {
        attribute("p:presentation")
    } else if name == "xl/workbook.xml" {
        attribute("workbook")
    } else {
        attribute("worksheet")
    }
}

/// A part named `name` declaring and holding exactly [`BOMB`] bytes, the
/// letter `a` filling what its markup leaves.
pub(crate) fn bomb_part(name: &str) -> RawPart {
    let (open, close) = bomb_markup(name);
    let n = BOMB as usize - open.len() - close.len();
    let (data, crc) = deflate_run(&open, b'a', n, &close);
    RawPart { name: name.to_string(), declared: BOMB, crc: crc.sum(), data }
}

/// A part named `name` of exactly `total` bytes: `xml_head` (one complete
/// document) with the letter `a` added as the value of an attribute `x` on its
/// root element, so the padding is never text (SPEC-0088 DEC-011).
pub(crate) fn padded_part(name: &str, xml_head: &str, total: usize) -> RawPart {
    let at = xml_head[1..].find([' ', '>', '/']).expect("a root element") + 1;
    let (open, close) = (format!("{} x=\"", &xml_head[..at]), format!("\"{}", &xml_head[at..]));
    let n = total.checked_sub(open.len() + close.len()).expect("total holds the markup");
    let (data, crc) = deflate_run(&open, b'a', n, &close);
    RawPart { name: name.to_string(), declared: total as u64, crc: crc.sum(), data }
}

/// A process wide lock taken first by every test that builds or decodes a part
/// of 10 MB or more: each can hold about 1 GB, so they must run one at a time.
pub(crate) async fn heavy() -> tokio::sync::MutexGuard<'static, ()> {
    static HEAVY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    HEAVY.lock().await
}

/// A project with an empty `/docs` folder (SPEC-0088).
pub(crate) async fn office_fixture() -> Fixture {
    let f = Fixture::with_config(|_| {}).await;
    f.seed_project(MOUNT, OWNER).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/docs", true).await.unwrap();
    f
}

/// [`office_fixture`] with a session write allowance of `n` bytes.
pub(crate) async fn office_fixture_allowance(n: i64) -> Fixture {
    let f = Fixture::with_config(move |c| c.safety.write_quota_bytes = n).await;
    f.seed_project(MOUNT, OWNER).await;
    let c = f.state.stores.client(MOUNT).await.unwrap();
    c.makedirs("/docs", true).await.unwrap();
    f
}

/// A zip of `parts` (name, content), each deflated and declaring its real size.
pub(crate) fn office_zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let raw: Vec<RawPart> = parts.iter().map(|(name, content)| raw_part(name, content)).collect();
    zip_raw(&raw)
}

/// Authorizes then extracts the text with the assistant tool's defaults, as both doors do.
pub(crate) async fn extract_text(
    f: &Fixture,
    person: &str,
    path: &str,
    refresh: bool,
) -> Result<Value> {
    f.state.authorize(MOUNT, person).await.unwrap();
    let c = f.state.stores.client(MOUNT).await.unwrap();
    crate::core::fs_ops::extract_document(
        &c,
        &f.state.safety,
        &f.state.config.extract.ocr,
        person,
        MOUNT,
        path,
        200_000,
        4_000,
        true,
        refresh,
    )
    .await
}

/// Forget every Office call measured on this thread.
pub(crate) fn office_held_reset() {
    crate::docs::extract::OFFICE_HELD.with(|m| m.borrow_mut().clear());
}

/// Decoded part bytes the extraction of `path` held, 0 when it read no part.
pub(crate) fn parts_held(path: &str) -> u64 {
    crate::docs::extract::OFFICE_HELD.with(|m| m.borrow().get(path).map_or(0, |h| h.parts))
}

/// Nothing the refused extraction of `doc` could leave is there: no companion
/// (or `companion_before` byte for byte), no artifact, no audit line, and the
/// whole default session allowance is still free.
pub(crate) async fn nothing_left(f: &Fixture, doc: &str, companion_before: Option<&[u8]>) {
    let md = crate::docs::extract::companion_md_path(doc);
    let c = f.state.stores.client(MOUNT).await.unwrap();
    match companion_before {
        None => {
            let listing =
                crate::core::fs_ops::list_dir(&c, "/docs", true, "name", false).await.unwrap();
            let name = |p: &str| format!("\"{}\"", p.rsplit('/').next().unwrap());
            assert!(listing.to_string().contains(&name(doc)), "{listing}");
            assert!(!listing.to_string().contains(&name(&md)), "{listing}");
        }
        Some(before) => assert_eq!(read(f, &md).await.as_deref(), Some(before)),
    }
    for listed in [
        crate::tools::artifacts::list_tables(&f.state, MOUNT, doc, None).await,
        crate::tools::artifacts::list_images(&f.state, MOUNT, doc, None).await,
    ] {
        let e = listed.expect_err("no artifact may be listed");
        assert_eq!(
            (e.code, e.message.as_str()),
            (code::NOT_FOUND, crate::docs::artifacts::NO_ARTIFACTS)
        );
    }
    let log = f.state.safety.audit(OWNER, MOUNT);
    assert!(!log.iter().any(|e| e.op == "extract_text"), "{log:?}");
    crate::core::fs_ops::write_bytes(
        &c,
        &f.state.safety,
        OWNER,
        MOUNT,
        "/docs/fill.bin",
        &vec![b'w'; 52_428_800],
        true,
        true,
    )
    .await
    .expect("nothing was charged");
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
