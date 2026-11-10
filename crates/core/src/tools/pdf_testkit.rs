//! PDF fixture builders and hook readers of the SPEC-0089 tests (DEC-019). The
//! documents are written by hand, byte by byte, because lopdf's writer cannot
//! write a crafted object stream nor a compressed cross reference entry; a bomb
//! stream reuses the cached deflate segments of `archive_testkit`, so a stream
//! of 200 000 000 spaces costs about 200 KB and no 200 MB compression.

use crate::tools::archive_testkit::deflate_run;

/// Bytes of a PDF bomb stream once inflated (SPEC-0089 section 9).
pub(crate) const PDF_BOMB: usize = 200_000_000;

/// The stream limit of SPEC-0089 FR-NEW-002.
pub(crate) const STREAM_LIMIT: usize = 104_857_600;

/// Most decoded bytes a bombed extraction may hold: the stream limit, one
/// measuring chunk past it, and 10 000 bytes of small streams (SPEC-0089 section 9).
pub(crate) const BOMB_HELD: u64 = 104_933_136;

/// The Adler 32 of `prefix`, `n` copies of `letter`, then `suffix`. The run is
/// added in closed form, so 200 000 000 letters cost no pass over them.
fn adler32(prefix: &[u8], letter: u8, n: usize, suffix: &[u8]) -> u32 {
    const MOD: u128 = 65_521;
    let (mut a, mut b) = (1u128, 0u128);
    let bytes = |data: &[u8], a: &mut u128, b: &mut u128| {
        for &c in data {
            *a = (*a + u128::from(c)) % MOD;
            *b = (*b + *a) % MOD;
        }
    };
    bytes(prefix, &mut a, &mut b);
    // After n copies of c: a grows by n c, and b by n a plus c n (n + 1) / 2.
    let (n, c) = (n as u128, u128::from(letter));
    let a_run = (a + n * c) % MOD;
    b = (b + n * a + c * (n * (n + 1) / 2)) % MOD;
    a = a_run;
    bytes(suffix, &mut a, &mut b);
    ((b as u32) << 16) | a as u32
}

/// The zlib stream of `prefix`, `n` copies of `letter`, then `suffix`: the zlib
/// header, the cached raw deflate of `deflate_run`, then the Adler 32 checksum.
pub(crate) fn zlib_run(prefix: &str, letter: u8, n: usize, suffix: &str) -> Vec<u8> {
    let (deflate, _) = deflate_run(prefix, letter, n, suffix);
    let mut out = Vec::with_capacity(deflate.len() + 6);
    out.extend_from_slice(&[0x78, 0x9C]);
    out.extend_from_slice(&deflate);
    out.extend_from_slice(&adler32(prefix.as_bytes(), letter, n, suffix.as_bytes()).to_be_bytes());
    out
}

/// One indirect object of a [`PdfWriter`]: a dictionary or other direct object
/// written as text, or a stream given by its dictionary text and stored bytes.
#[derive(Clone)]
pub(crate) enum PdfObj {
    Body(String),
    Stream(String, Vec<u8>),
}

/// A stream object of `content` stored as is, with its real `/Length`.
pub(crate) fn plain(content: &str) -> PdfObj {
    PdfObj::Stream(format!("<< /Length {} >>", content.len()), content.as_bytes().to_vec())
}

/// A stream object of the zlib bytes `stored`, `/FlateDecode`, `dict_extra` added to its dictionary.
pub(crate) fn flate(dict_extra: &str, stored: Vec<u8>) -> PdfObj {
    PdfObj::Stream(
        format!("<< {dict_extra} /Length {} /Filter /FlateDecode >>", stored.len()),
        stored,
    )
}

/// The dictionary text and stored bytes of a PDF bomb stream: exactly
/// [`PDF_BOMB`] spaces, zlib, `/Length` its real stored length.
pub(crate) fn bomb_stream(dict_extra: &str) -> (String, Vec<u8>) {
    let stored = zlib_run("", b' ', PDF_BOMB, "");
    assert!(stored.len() < 300_000, "a bomb stream is stored in under 300 000 bytes");
    (format!("<< {dict_extra} /Length {} /Filter /FlateDecode >>", stored.len()), stored)
}

/// [`bomb_stream`] as an object.
pub(crate) fn bomb(dict_extra: &str) -> PdfObj {
    let (dict, stored) = bomb_stream(dict_extra);
    PdfObj::Stream(dict, stored)
}

/// The content of an object stream holding `objects`: the `n off` pairs, the
/// objects, then `padding` spaces; returns the text before the padding and `/First`.
fn object_stream_head(objects: &[(u32, &str)]) -> (String, usize) {
    let mut pairs = String::new();
    let mut body = String::new();
    for (n, text) in objects {
        pairs.push_str(&format!("{n} {} ", body.len()));
        body.push_str(text);
        body.push(' ');
    }
    let first = pairs.len();
    (pairs + &body, first)
}

/// A PDF of numbered objects, written with a classic cross reference, or with
/// a cross reference stream of uncompressed `/W [1 4 2]` entries when some
/// objects sit in object streams.
pub(crate) struct PdfWriter {
    objects: std::collections::BTreeMap<u32, PdfObj>,
    /// Object number to (container, index) for the compressed objects.
    compressed: std::collections::BTreeMap<u32, (u32, u16)>,
    root: u32,
    info: Option<u32>,
}

impl PdfWriter {
    pub(crate) fn new(root: u32) -> Self {
        Self { objects: Default::default(), compressed: Default::default(), root, info: None }
    }

    /// Set (or replace) object `n`.
    pub(crate) fn object(&mut self, n: u32, obj: PdfObj) -> &mut Self {
        self.objects.insert(n, obj);
        self
    }

    pub(crate) fn info(&mut self, n: u32) -> &mut Self {
        self.info = Some(n);
        self
    }

    /// Object stream `number` holding `objects`, then `padding` spaces, deflated.
    pub(crate) fn object_stream(
        &mut self,
        number: u32,
        objects: &[(u32, &str)],
        padding: usize,
    ) -> &mut Self {
        let (head, first) = object_stream_head(objects);
        let stored = zlib_run(&head, b' ', padding, "");
        let dict = format!(
            "<< /Type /ObjStm /N {} /First {first} /Filter /FlateDecode /Length {} >>",
            objects.len(),
            stored.len()
        );
        self.objects.insert(number, PdfObj::Stream(dict, stored));
        for (index, (n, _)) in objects.iter().enumerate() {
            self.compressed.insert(*n, (number, index as u16));
        }
        self
    }

    pub(crate) fn finish(&self) -> Vec<u8> {
        let mut out = b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = std::collections::BTreeMap::new();
        for (n, obj) in &self.objects {
            offsets.insert(*n, out.len());
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            match obj {
                PdfObj::Body(text) => out.extend_from_slice(text.as_bytes()),
                PdfObj::Stream(dict, stored) => {
                    out.extend_from_slice(dict.as_bytes());
                    out.extend_from_slice(b"\nstream\n");
                    out.extend_from_slice(stored);
                    out.extend_from_slice(b"\nendstream");
                }
            }
            out.extend_from_slice(b"\nendobj\n");
        }
        let info = self.info.map(|i| format!(" /Info {i} 0 R")).unwrap_or_default();
        let last = self.objects.keys().chain(self.compressed.keys()).max().copied().unwrap_or(0);
        if self.compressed.is_empty() {
            let size = last + 1;
            let xref = out.len();
            out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
            for n in 0..size {
                match offsets.get(&n) {
                    Some(off) => out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes()),
                    None => out.extend_from_slice(b"0000000000 65535 f \n"),
                }
            }
            out.extend_from_slice(
                format!(
                    "trailer\n<< /Size {size} /Root {} 0 R{info} >>\nstartxref\n{xref}\n%%EOF\n",
                    self.root
                )
                .as_bytes(),
            );
        } else {
            let xref_no = last + 1;
            let size = xref_no + 1;
            let xref = out.len();
            offsets.insert(xref_no, xref);
            let mut entries = Vec::new();
            for n in 0..size {
                let (kind, field2, field3) = if let Some(off) = offsets.get(&n) {
                    (1u8, *off as u32, 0u16)
                } else if let Some((container, index)) = self.compressed.get(&n) {
                    (2u8, *container, *index)
                } else {
                    (0u8, 0, if n == 0 { 0xFFFF } else { 0 })
                };
                entries.push(kind);
                entries.extend_from_slice(&field2.to_be_bytes());
                entries.extend_from_slice(&field3.to_be_bytes());
            }
            out.extend_from_slice(
                format!(
                    "{xref_no} 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] /Root {} 0 R{info} /Length {} >>\nstream\n",
                    self.root,
                    entries.len()
                )
                .as_bytes(),
            );
            out.extend_from_slice(&entries);
            out.extend_from_slice(
                format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
            );
        }
        out
    }
}

/// The objects of a document of pages showing `contents` (object 4 for page 1,
/// 6 for page 2), each page with `resources` added to its resource dictionary,
/// and `/F1` a `Helvetica` font (object 5): catalog 1, page tree 2, page 1 is 3,
/// page 2 is 7. Callers replace or add objects before `finish`.
pub(crate) fn pages_writer(contents: Vec<PdfObj>, resources: &str) -> PdfWriter {
    let mut w = PdfWriter::new(1);
    let page_ids: Vec<u32> = [3, 7].into_iter().take(contents.len()).collect();
    let kids: Vec<String> = page_ids.iter().map(|p| format!("{p} 0 R")).collect();
    w.object(1, PdfObj::Body("<< /Type /Catalog /Pages 2 0 R >>".into()));
    w.object(
        2,
        PdfObj::Body(format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        )),
    );
    w.object(5, PdfObj::Body("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into()));
    for (page, (content_no, content)) in page_ids.iter().zip([4u32, 6].into_iter().zip(contents)) {
        w.object(
            *page,
            PdfObj::Body(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> {resources} >> /Contents {content_no} 0 R >>"
            )),
        );
        w.object(content_no, content);
    }
    w
}

/// A one page document showing `content`, `resources` added to its resource
/// dictionary, `extra` objects set (and replacing) by number.
pub(crate) fn page_pdf(content: PdfObj, resources: &str, extra: Vec<(u32, PdfObj)>) -> Vec<u8> {
    let mut w = pages_writer(vec![content], resources);
    for (n, obj) in extra {
        w.object(n, obj);
    }
    small(w.finish())
}

/// `doc`, checked to be stored in under 1 000 000 bytes (SPEC-0089 section 9).
pub(crate) fn small(doc: Vec<u8>) -> Vec<u8> {
    assert!(
        doc.len() < 1_000_000,
        "a bomb document is stored in under 1 000 000 bytes: {}",
        doc.len()
    );
    doc
}

/// SEC-001: page 1's content is the bomb stream.
pub(crate) fn content_bomb_pdf() -> Vec<u8> {
    page_pdf(bomb(""), "", vec![])
}

/// SEC-002: object 1 `<< /Pad true >>` sits in object stream 5, padded with
/// spaces to exactly [`PDF_BOMB`] bytes.
pub(crate) fn object_stream_bomb_pdf() -> Vec<u8> {
    let mut w = pages_writer(vec![plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET")], "");
    // The catalog moves to 8 so that object 1 can be the compressed one.
    w.objects.remove(&1);
    w.root = 8;
    w.object(8, PdfObj::Body("<< /Type /Catalog /Pages 2 0 R >>".into()));
    let objects = [(1, "<< /Pad true >>")];
    let (head, _) = object_stream_head(&objects);
    w.object_stream(9, &objects, PDF_BOMB - head.len());
    small(w.finish())
}

/// SEC-003: `/F1` is a Type 1 font whose descriptor's `/FontFile` is the bomb.
pub(crate) fn font_file_bomb_pdf() -> Vec<u8> {
    page_pdf(
        plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET"),
        "",
        vec![
            (
                5,
                PdfObj::Body(
                    "<< /Type /Font /Subtype /Type1 /BaseFont /Bomb /FontDescriptor 6 0 R >>"
                        .into(),
                ),
            ),
            (6, PdfObj::Body("<< /Type /FontDescriptor /FontFile 7 0 R >>".into())),
            (7, bomb("")),
        ],
    )
}

/// SEC-004: `/F1`'s `/ToUnicode` is the bomb.
pub(crate) fn cmap_bomb_pdf() -> Vec<u8> {
    page_pdf(
        plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET"),
        "",
        vec![
            (
                5,
                PdfObj::Body(
                    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 7 0 R >>"
                        .into(),
                ),
            ),
            (7, bomb("")),
        ],
    )
}

/// SEC-005: page 1 runs `/CS0 cs`, `/CS0` an ICC profile that is the bomb.
pub(crate) fn colour_profile_bomb_pdf() -> Vec<u8> {
    page_pdf(plain("/CS0 cs"), "/ColorSpace << /CS0 [/ICCBased 7 0 R] >>", vec![(7, bomb("/N 1"))])
}

/// SEC-006: page 1 draws `/Im0` once, an image over the bomb.
pub(crate) fn image_bomb_pdf() -> Vec<u8> {
    page_pdf(
        plain("/Im0 Do"),
        "/XObject << /Im0 7 0 R >>",
        vec![(
            7,
            bomb(
                "/Subtype /Image /Width 10000 /Height 20000 /BitsPerComponent 8 /ColorSpace /DeviceGray",
            ),
        )],
    )
}

/// SEC-012 and E2E-032: the two pages of E2E-001 (`Quarterly results`, then
/// `Outlook`) whose `/Info` (object 8, title `Report 2026`) sits in object
/// stream 9, padded to exactly [`PDF_BOMB`] bytes.
pub(crate) fn titled_bomb_pdf() -> Vec<u8> {
    let mut w = pages_writer(
        vec![
            plain("BT /F1 12 Tf 72 700 Td (Quarterly results) Tj ET"),
            plain("BT /F1 12 Tf 72 700 Td (Outlook) Tj ET"),
        ],
        "",
    );
    let objects = [(8, "<< /Title (Report 2026) >>")];
    let (head, _) = object_stream_head(&objects);
    w.object_stream(9, &objects, PDF_BOMB - head.len());
    w.info(8);
    small(w.finish())
}

/// E2E-007 and E2E-008: page 1 shows 250 000 letters `b`, its content padded
/// with spaces to exactly `total` bytes once inflated.
pub(crate) fn large_content_pdf(total: usize) -> Vec<u8> {
    let head = format!("BT /F1 12 Tf 72 700 Td ({}) Tj ET", "b".repeat(250_000));
    page_pdf(flate("", zlib_run(&head, b' ', total - head.len(), "")), "", vec![])
}

/// E2E-017: page 1 shows `Caption` then draws 200 times a 900 000 byte image
/// in `/DCTDecode`, an encoding the extraction does not decode.
pub(crate) fn photo_pdf() -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 700 Td (Caption) Tj ET{}", " /Im0 Do".repeat(200));
    let mut image = vec![0xFF, 0xD8];
    image.extend(std::iter::repeat_n(b'x', 899_998));
    page_pdf(
        plain(&content),
        "/XObject << /Im0 7 0 R >>",
        vec![(
            7,
            PdfObj::Stream(
                format!(
                    "<< /Type /XObject /Subtype /Image /Width 100 /Height 100 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /DCTDecode /Length {} >>",
                    image.len()
                ),
                image,
            ),
        )],
    )
}

/// Forget every PDF call measured on this thread.
pub(crate) fn pdf_held_reset() {
    crate::docs::extract::PDF_HELD.with(|m| m.borrow_mut().clear());
}

/// Decoded stream bytes the extraction of `path` held at most. Panics when the
/// extraction recorded no figures, so a missing record never reads as 0.
pub(crate) fn decoded_held(path: &str) -> u64 {
    crate::docs::extract::PDF_HELD
        .with(|m| m.borrow().get(path).map(|h| h.decoded))
        .unwrap_or_else(|| panic!("the extraction of {path} recorded no figures"))
}

/// Largest content buffer the extraction of `path` built, 0 when it built none.
#[allow(dead_code)] // read by the content limit tests of US-0216
pub(crate) fn pdf_content_held(path: &str) -> u64 {
    crate::docs::extract::PDF_HELD.with(|m| m.borrow().get(path).map_or(0, |h| h.content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn bomb_documents_are_small() {
        for doc in [
            content_bomb_pdf(),
            object_stream_bomb_pdf(),
            font_file_bomb_pdf(),
            cmap_bomb_pdf(),
            colour_profile_bomb_pdf(),
            image_bomb_pdf(),
            titled_bomb_pdf(),
        ] {
            assert!(doc.len() < 1_000_000);
        }
    }

    #[test]
    fn zlib_run_matches_flate2() {
        let stored = zlib_run("<<", b' ', 2_500_000, ">>");
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(stored.as_slice()).read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 2_500_004);
        assert_eq!(&out[..2], b"<<");
        assert!(out[2..2_500_002].iter().all(|b| *b == b' '));
        assert_eq!(&out[2_500_002..], b">>");
    }
}
