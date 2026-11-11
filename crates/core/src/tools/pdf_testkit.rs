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

/// `data` as one zlib stream, compressed for real (a few megabytes at most).
fn zlib(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(data).expect("a vector takes every write");
    encoder.finish().expect("a vector takes every write")
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

    /// Object stream `number` holding `count` copies of the pair `pair` (all at
    /// offset 0 of the one object `object`), deflated. The pairs are not listed
    /// in the cross reference: the stream is the attacker's claim of objects.
    pub(crate) fn object_stream_pairs(
        &mut self,
        number: u32,
        pair: (u32, u32),
        count: usize,
        object: &str,
    ) -> &mut Self {
        let unit = format!("{} {} ", pair.0, pair.1);
        let content = format!("{}{object}", unit.repeat(count));
        let dict = |len: usize| {
            format!(
                "<< /Type /ObjStm /N {count} /First {} /Filter /FlateDecode /Length {len} >>",
                unit.len() * count
            )
        };
        let stored = zlib(content.as_bytes());
        self.objects.insert(number, PdfObj::Stream(dict(stored.len()), stored));
        self
    }

    /// Object stream `number` holding `count` `null` objects numbered `first`
    /// upwards, each listed in the cross reference.
    pub(crate) fn object_stream_nulls(&mut self, number: u32, first: u32, count: u32) -> &mut Self {
        let mut pairs = String::new();
        for i in 0..count {
            pairs.push_str(&format!("{} {} ", first + i, 5 * i));
        }
        let content = format!("{pairs}{}", "null ".repeat(count as usize));
        let stored = zlib(content.as_bytes());
        let dict = format!(
            "<< /Type /ObjStm /N {count} /First {} /Filter /FlateDecode /Length {} >>",
            pairs.len(),
            stored.len()
        );
        self.objects.insert(number, PdfObj::Stream(dict, stored));
        for i in 0..count {
            self.compressed.insert(first + i, (number, i as u16));
        }
        self
    }

    pub(crate) fn finish(&self) -> Vec<u8> {
        self.finish_as(self.compressed.is_empty())
    }

    /// The document with a classic cross reference whatever it holds: the
    /// compressed objects are listed as free, as before a later section adds them.
    fn finish_classic(&self) -> Vec<u8> {
        self.finish_as(true)
    }

    fn finish_as(&self, classic: bool) -> Vec<u8> {
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
        if classic {
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

/// The zlib stream of the bytes `head`, then `n` copies of `letter`: `head` in
/// a stored deflate block (it may be binary), then the cached run.
fn zlib_head_run(head: &[u8], letter: u8, n: usize) -> Vec<u8> {
    let (run, _) = deflate_run("", letter, n, "");
    let len = u16::try_from(head.len()).expect("a stored block holds at most 65 535 bytes");
    let mut out = vec![0x78, 0x9C, 0x00];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(head);
    out.extend_from_slice(&run);
    out.extend_from_slice(&adler32(head, letter, n, b"").to_be_bytes());
    out
}

/// The LZW code of `data` as lopdf decodes it by default (`EarlyChange` 1).
pub(crate) fn lzw(data: &[u8]) -> Vec<u8> {
    weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8).encode(data).unwrap()
}

/// The LZW code of `n` copies of `letter`: weezl's encoder fed a 1 000 000 byte
/// buffer repeated, computed once per process and arguments.
pub(crate) fn lzw_run(letter: u8, n: usize) -> Vec<u8> {
    use std::io::Read as _;
    type Codes = Vec<((u8, usize), Vec<u8>)>;
    static CACHE: std::sync::Mutex<Codes> = std::sync::Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap();
    if let Some((_, code)) = cache.iter().find(|(k, _)| *k == (letter, n)) {
        return code.clone();
    }
    let source = std::io::repeat(letter).take(n as u64);
    let reader = std::io::BufReader::with_capacity(1_000_000, source);
    let mut code = Vec::new();
    let mut encoder = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
    encoder.into_stream(&mut code).encode_all(reader).status.unwrap();
    cache.push(((letter, n), code.clone()));
    code
}

/// The ASCII85 form of `data`, ended by `~>`.
pub(crate) fn ascii85(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 * 5 + 8);
    for group in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..group.len()].copy_from_slice(group);
        let mut value = u32::from_be_bytes(word);
        if group.len() == 4 && value == 0 {
            out.push(b'z');
            continue;
        }
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = (value % 85) as u8 + b'!';
            value /= 85;
        }
        out.extend_from_slice(&digits[..group.len() + 1]);
    }
    out.extend_from_slice(b"~>");
    out
}

/// A one page document whose only cross reference is a stream of its six
/// entries followed by spaces up to [`PDF_BOMB`] decoded bytes.
pub(crate) fn xref_stream_bomb_pdf() -> Vec<u8> {
    let w = pages_writer(vec![plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET")], "");
    // The classic form gives the objects' offsets; its cross reference is cut off.
    let classic = w.finish_classic();
    let body_end = find(&classic, b"xref\n");
    let mut out = classic[..body_end].to_vec();
    let mut entries = vec![0u8, 0, 0, 0, 0, 0xFF, 0xFF];
    for n in 1..6u32 {
        let off = find(&classic, format!("\n{n} 0 obj\n").as_bytes()) + 1;
        entries.push(1);
        entries.extend_from_slice(&(off as u32).to_be_bytes());
        entries.extend_from_slice(&0u16.to_be_bytes());
    }
    let stored = zlib_head_run(&entries, b' ', PDF_BOMB - entries.len());
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "6 0 obj\n<< /Type /XRef /Size 6 /W [1 4 2] /Root 1 0 R /Filter /FlateDecode /Length {} >>\nstream\n",
            stored.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&stored);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    small(out)
}

fn find(data: &[u8], needle: &[u8]) -> usize {
    data.windows(needle.len()).position(|w| w == needle).expect("present")
}

/// The load filter of the fixture builder: object streams kept whole.
fn keep_object_streams(
    id: pdf_extract::ObjectId,
    object: &mut pdf_extract::Object,
) -> Option<(pdf_extract::ObjectId, pdf_extract::Object)> {
    if let pdf_extract::Object::Stream(stream) = object
        && stream.dict.has_type(b"ObjStm")
    {
        stream.dict.set("Type", pdf_extract::Object::Name(b"FixtureObjStm".to_vec()));
    }
    Some((id, pdf_extract::Object::Null))
}

/// `w` encrypted by lopdf (`EncryptionVersion::V1`, user password empty), then
/// a cross reference stream appended that lists the objects of its object
/// streams as compressed, `/Prev` the classic section lopdf wrote. The new
/// main trailer names the encryption dictionary with `encrypt_key`, the
/// spelling of `/Encrypt` to write (SPEC-0089 DEC-019).
pub(crate) fn encrypted(w: &PdfWriter, encrypt_key: &str) -> Vec<u8> {
    encrypted_with(w, encrypt_key, "", false, |_| {})
}

/// [`encrypted`] with `user_password` as the user password, AES 128
/// (`EncryptionVersion::V4`) when `aes`, and `tamper` applied to the
/// document once encrypted, before it is written.
fn encrypted_with(
    w: &PdfWriter,
    encrypt_key: &str,
    user_password: &str,
    aes: bool,
    tamper: fn(&mut pdf_extract::Document),
) -> Vec<u8> {
    use pdf_extract::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
    use pdf_extract::{Document, EncryptionState, EncryptionVersion, LoadOptions, Object};
    let options = LoadOptions { filter: Some(keep_object_streams), ..Default::default() };
    let mut doc = Document::load_mem_with_options(&w.finish_classic(), options).unwrap();
    let id = b"0123456789abcdef".to_vec();
    doc.trailer.set(
        "ID",
        Object::Array(vec![
            Object::String(id.clone(), pdf_extract::StringFormat::Hexadecimal),
            Object::String(id.clone(), pdf_extract::StringFormat::Hexadecimal),
        ]),
    );
    let version = if aes {
        let filter: std::sync::Arc<dyn CryptFilter> = std::sync::Arc::new(Aes128CryptFilter);
        EncryptionVersion::V4 {
            document: &doc,
            encrypt_metadata: true,
            crypt_filters: std::collections::BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: "owner",
            user_password,
            permissions: Default::default(),
        }
    } else {
        EncryptionVersion::V1 {
            document: &doc,
            owner_password: "owner",
            user_password,
            permissions: Default::default(),
        }
    };
    let state = EncryptionState::try_from(version).unwrap();
    doc.encrypt(&state).unwrap();
    tamper(&mut doc);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    // lopdf's writer leaves object streams out, so they wear another type until
    // written; names are not encrypted, and spaces keep every offset.
    let renamed = b"/FixtureObjStm";
    while let Some(at) = out.windows(renamed.len()).position(|w| w == renamed) {
        out[at..at + renamed.len()].copy_from_slice(b"/ObjStm       ");
    }

    let at = out.windows(9).rposition(|w| w == b"startxref").unwrap() + 9;
    let tail = std::str::from_utf8(&out[at..]).unwrap();
    let prev: usize = tail.split_whitespace().next().unwrap().parse().unwrap();
    let reference = |key: &[u8]| {
        doc.trailer.get(key).and_then(Object::as_reference).map(|(n, g)| format!("{n} {g} R"))
    };
    let encrypt = reference(b"Encrypt").unwrap();
    let root = reference(b"Root").unwrap();
    let info = reference(b"Info").map(|i| format!(" /Info {i}")).unwrap_or_default();
    let number = doc.max_id + 1;
    let mut index = String::new();
    let mut entries = Vec::new();
    for (n, (container, i)) in &w.compressed {
        index.push_str(&format!("{n} 1 "));
        entries.push(2u8);
        entries.extend_from_slice(&container.to_be_bytes());
        entries.extend_from_slice(&i.to_be_bytes());
    }
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    out.push(b'\n');
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "{number} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Index [{}] /Prev {prev} /Root {root}{info} {encrypt_key} {encrypt} /ID [<{hex}> <{hex}>] /Length {} >>\nstream\n",
            number + 1,
            index.trim_end(),
            entries.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&entries);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

/// The writer of SEC-002: object 1 `<< /Pad true >>` sits in object stream 9,
/// padded with spaces to exactly `total` decoded bytes.
fn object_stream_writer(total: usize) -> PdfWriter {
    let mut w = pages_writer(vec![plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET")], "");
    // The catalog moves to 8 so that object 1 can be the compressed one.
    w.objects.remove(&1);
    w.root = 8;
    w.object(8, PdfObj::Body("<< /Type /Catalog /Pages 2 0 R >>".into()));
    let objects = [(1, "<< /Pad true >>")];
    let (head, _) = object_stream_head(&objects);
    w.object_stream(9, &objects, total - head.len());
    w
}

/// The document of SEC-002 encrypted with an empty user password.
pub(crate) fn locked_bomb_pdf() -> Vec<u8> {
    small(encrypted(&object_stream_writer(PDF_BOMB), "/Encrypt"))
}

/// The two pages of E2E-001 with `/Info` (title `Report 2026`) in object stream 9.
fn titled_writer(padding_to: usize) -> PdfWriter {
    let mut w = pages_writer(
        vec![
            plain("BT /F1 12 Tf 72 700 Td (Quarterly results) Tj ET"),
            plain("BT /F1 12 Tf 72 700 Td (Outlook) Tj ET"),
        ],
        "",
    );
    let objects = [(8, "<< /Title (Report 2026) >>")];
    let (head, _) = object_stream_head(&objects);
    w.object_stream(9, &objects, padding_to.saturating_sub(head.len()));
    w.info(8);
    w
}

/// The two page titled document encrypted with an empty user password, its
/// main trailer naming the encryption dictionary `encrypt_key`.
pub(crate) fn secret_pdf(encrypt_key: &str) -> Vec<u8> {
    encrypted(&titled_writer(0), encrypt_key)
}

/// The two page titled document encrypted with the user password `user`.
pub(crate) fn passworded_pdf() -> Vec<u8> {
    encrypted_with(&titled_writer(0), "/Encrypt", "user", false, |_| {})
}

/// The two page titled document encrypted with AES 128 and an empty user
/// password; when `bad_string`, the page tree (object 2) also holds a string
/// of 5 bytes, which no AES decryption accepts.
pub(crate) fn aes_secret_pdf(bad_string: bool) -> Vec<u8> {
    let tamper: fn(&mut pdf_extract::Document) = if bad_string {
        |doc| {
            let pages = doc.get_dictionary_mut((2, 0)).unwrap();
            pages.set("Junk", pdf_extract::Object::string_literal("short"));
        }
    } else {
        |_| {}
    };
    encrypted_with(&titled_writer(0), "/Encrypt", "", true, tamper)
}

/// Page 1's content is the LZW code of [`PDF_BOMB`] spaces.
pub(crate) fn lzw_bomb_pdf() -> Vec<u8> {
    let code = lzw_run(b' ', PDF_BOMB);
    let dict = format!("<< /Filter /LZWDecode /Length {} >>", code.len());
    page_pdf(PdfObj::Stream(dict, code), "", vec![])
}

/// Page 1's content is the ASCII85 form of the zlib bomb stream.
pub(crate) fn ascii85_bomb_pdf() -> Vec<u8> {
    let stored = ascii85(&zlib_run("", b' ', PDF_BOMB, ""));
    let dict = format!("<< /Filter [/ASCII85Decode /FlateDecode] /Length {} >>", stored.len());
    page_pdf(PdfObj::Stream(dict, stored), "", vec![])
}

/// Two pages: `Old LZW` under `/LZWDecode`, `Old A85` under `/ASCII85Decode`.
pub(crate) fn old_filters_pdf() -> Vec<u8> {
    let page = |text: &str| format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET");
    let one = lzw(page("Old LZW").as_bytes());
    let two = ascii85(page("Old A85").as_bytes());
    let w = pages_writer(
        vec![
            PdfObj::Stream(format!("<< /Filter /LZWDecode /Length {} >>", one.len()), one),
            PdfObj::Stream(format!("<< /Filter /ASCII85Decode /Length {} >>", two.len()), two),
        ],
        "",
    );
    small(w.finish())
}

/// Most decoded bytes a call may hold: the load total, one measuring chunk
/// past it, and 10 000 bytes of small streams (SPEC-0089 section 9).
pub(crate) const LOAD_HELD: u64 = 268_500_992;

/// A document of one page reading `text`, whose objects 30, 31 and 32 sit in
/// object streams 20, 21 and 22 inflating to `totals` bytes each (E2E-009,
/// E2E-010, E2E-SEC-009); the objects are padded with spaces.
pub(crate) fn object_streams_pdf(text: &str, totals: [usize; 3]) -> Vec<u8> {
    let mut w = pages_writer(vec![plain(&format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET"))], "");
    for (i, total) in totals.into_iter().enumerate() {
        let objects = [(30 + i as u32, "<< /Pad 1 >>")];
        let (head, _) = object_stream_head(&objects);
        w.object_stream(20 + i as u32, &objects, total - head.len());
    }
    w.finish()
}

/// Objects the load counts outside the padding of [`many_pdf`]: the catalog,
/// the page tree, the page, its content and its font, the padding's object
/// stream and the cross reference stream.
const MANY_OUTSIDE: u32 = 7;

/// E2E-011 and E2E-012: one page reading `Many`, and `null` objects with
/// distinct numbers in one object stream, so that the document holds exactly
/// 1 000 000 objects when `over` is 0 and 1 000 001 when it is 1.
pub(crate) fn many_pdf(over: u32) -> Vec<u8> {
    let mut w = pages_writer(vec![plain("BT /F1 12 Tf 72 700 Td (Many) Tj ET")], "");
    w.object_stream_nulls(20, 100, 1_000_000 - MANY_OUTSIDE + over);
    w.finish()
}

/// E2E-026: object stream 5 holds 1 000 001 pairs `1 0` over one `null`
/// (within 10 000 000 decoded bytes), object stream 9 is a PDF bomb stream.
pub(crate) fn order_pdf() -> Vec<u8> {
    let mut w = pages_writer(vec![plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET")], "");
    w.object_stream_pairs(5, (1, 0), 1_000_001, "null");
    w.object(9, bomb("/Type /ObjStm /N 1 /First 4"));
    small(w.finish())
}

/// E2E-SEC-010: object stream 20 holds 2 000 000 pairs `1 0` over one `null`.
pub(crate) fn nulls_pdf() -> Vec<u8> {
    let mut w = pages_writer(vec![plain("BT /F1 12 Tf 72 700 Td (Hi) Tj ET")], "");
    w.object_stream_pairs(20, (1, 0), 2_000_000, "null");
    small(w.finish())
}

/// SEC-001: page 1's content is the bomb stream.
pub(crate) fn content_bomb_pdf() -> Vec<u8> {
    page_pdf(bomb(""), "", vec![])
}

/// SEC-002: object 1 `<< /Pad true >>` sits in object stream 5, padded with
/// spaces to exactly [`PDF_BOMB`] bytes.
pub(crate) fn object_stream_bomb_pdf() -> Vec<u8> {
    small(object_stream_writer(PDF_BOMB).finish())
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
    small(titled_writer(PDF_BOMB).finish())
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

/// The page total of SPEC-0089 FR-NEW-004.
pub(crate) const PAGE_LIMIT: usize = 268_435_456;

/// A form object (a `/Subtype /Form` stream, `/FlateDecode`) whose content is
/// `head` then spaces, `total` decoded bytes in all.
fn form_object(head: &str, total: usize) -> PdfObj {
    flate(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792]",
        zlib_run(head, b' ', total - head.len(), ""),
    )
}

/// A page content stream of `head` then spaces, `total` decoded bytes in all.
fn padded_content(head: &str, total: usize) -> PdfObj {
    flate("", zlib_run(head, b' ', total - head.len(), ""))
}

/// E2E-013 and E2E-014: page 1 draws form `/Fm0` (object 7, `Hi`) four times;
/// the page content and the four draws decode to exactly `PAGE_LIMIT + over`.
pub(crate) fn form_total_pdf(over: usize) -> Vec<u8> {
    const FORM: usize = 67_108_000;
    let page = PAGE_LIMIT + over - 4 * FORM;
    let form = form_object("BT /F1 12 Tf 72 700 Td (Hi) Tj ET", FORM);
    page_pdf(
        padded_content(&"/Fm0 Do ".repeat(4), page),
        "/XObject << /Fm0 7 0 R >>",
        vec![(7, form)],
    )
}

/// Page 1 draws three different forms (objects 7, 8 and 9): two of exactly the
/// stream limit each and a third of `third` bytes: with `PAGE_LIMIT - 2 *
/// STREAM_LIMIT + 1` the page total is met 1 byte past the end of a first
/// inflation, with 300 000 000 it is met 58 720 256 bytes into it.
pub(crate) fn big_forms_pdf(third: usize) -> Vec<u8> {
    let form = |total| form_object("BT /F1 12 Tf 72 700 Td (Hi) Tj ET", total);
    page_pdf(
        plain("/Fm0 Do /Fm1 Do /Fm2 Do"),
        "/XObject << /Fm0 7 0 R /Fm1 8 0 R /Fm2 9 0 R >>",
        vec![(7, form(STREAM_LIMIT)), (8, form(STREAM_LIMIT)), (9, form(third))],
    )
}

/// A form decoding to 1 000 000 bytes, drawn 300 times by one page content.
fn drawn_300_times() -> (PdfObj, PdfObj) {
    (plain(&"/Fm0 Do ".repeat(300)), form_object("BT /F1 12 Tf 72 700 Td (Hi) Tj ET", 1_000_000))
}

/// E2E-SEC-007: page 1 draws one 1 000 000 byte form 300 times.
pub(crate) fn forms_pdf() -> Vec<u8> {
    let (content, form) = drawn_300_times();
    page_pdf(content, "/XObject << /Fm0 7 0 R >>", vec![(7, form)])
}

/// E2E-018: page 1 reads `One`, page 2 draws a 1 000 000 byte form (object 8)
/// 300 times.
pub(crate) fn two_pages_pdf() -> Vec<u8> {
    let (content, form) = drawn_300_times();
    let mut w = pages_writer(
        vec![plain("BT /F1 12 Tf 72 700 Td (One) Tj ET"), content],
        "/XObject << /Fm0 8 0 R >>",
    );
    w.object(8, form);
    small(w.finish())
}

/// E2E-027: the object streams of E2E-010 (one byte over the load total) and a
/// page 1 drawing a 1 000 000 byte form 300 times.
pub(crate) fn order2_pdf() -> Vec<u8> {
    let (content, form) = drawn_300_times();
    let mut w = pages_writer(vec![content], "/XObject << /Fm0 8 0 R >>");
    w.object(8, form);
    for (i, total) in [100_000_000, 100_000_000, 68_435_457].into_iter().enumerate() {
        let objects = [(30 + i as u32, "<< /Pad 1 >>")];
        let (head, _) = object_stream_head(&objects);
        w.object_stream(20 + i as u32, &objects, total - head.len());
    }
    small(w.finish())
}

/// A page selecting `/CS0` (an ICC profile of 1 000 000 bytes, object 7) once,
/// then `q` `saves` times.
pub(crate) fn states_pdf(saves: usize) -> Vec<u8> {
    page_pdf(
        plain(&format!("/CS0 cs {}", "q ".repeat(saves))),
        "/ColorSpace << /CS0 [/ICCBased 7 0 R] >>",
        vec![(7, flate("/N 1", zlib_run("", b' ', 1_000_000, "")))],
    )
}

/// E2E-SEC-008: `/CS0 cs q` written 300 times over a 1 000 000 byte profile.
pub(crate) fn selected_states_pdf() -> Vec<u8> {
    page_pdf(
        plain(&"/CS0 cs q ".repeat(300)),
        "/ColorSpace << /CS0 [/ICCBased 7 0 R] >>",
        vec![(7, flate("/N 1", zlib_run("", b' ', 1_000_000, "")))],
    )
}

/// A font whose `/ToUnicode` (object 9) maps each code of `map` to its text:
/// every width is 0, so `PlainTextOutput` never inserts a space between two
/// codes. Returns the font object (5) and the character map object (9).
pub(crate) fn expanding_font(map: &[(u8, &str)]) -> Vec<(u32, PdfObj)> {
    let utf16 = |text: &str| text.encode_utf16().map(|u| format!("{u:04X}")).collect::<String>();
    let chars: String = map.iter().map(|(c, t)| format!("<{c:02X}> <{}>\n", utf16(t))).collect();
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Adobe-Identity-UCS def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange {} beginbfchar\n{chars}endbfchar endcmap CMapName currentdict /CMap defineresource pop end end",
        map.len()
    );
    vec![
        (
            5,
            PdfObj::Body(
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 1 /LastChar 3 /Widths [0 0 0] /ToUnicode 9 0 R >>"
                    .into(),
            ),
        ),
        (9, plain(&cmap)),
    ]
}

/// A page content stream showing `<01>` `ones` times then `last` (a hex
/// code), compressed for real (a few kilobytes).
fn expanding_content(ones: usize, last: &str) -> PdfObj {
    let text = format!("BT /F1 12 Tf 72 700 Td <{}{last}> Tj ET", "01".repeat(ones));
    flate("", zlib(text.as_bytes()))
}

/// The letters `e`, `n` of them.
fn letters(n: usize) -> String {
    "e".repeat(n)
}

/// E2E-015 and E2E-016: one page whose text, once the marker's leading
/// newlines are trimmed, is 16 + 268 435 200 + `last` bytes (`<02>` is mapped
/// to `last` letters `e`): 268 435 456 with 240.
pub(crate) fn expanding_pdf(last: usize) -> Vec<u8> {
    let map = [(1u8, letters(256)), (2u8, letters(last))];
    let map: Vec<(u8, &str)> = map.iter().map(|(c, t)| (*c, t.as_str())).collect();
    page_pdf(expanding_content(1_048_575, "02"), "", expanding_font(&map))
}

/// E2E-028: page 1 as [`expanding_pdf`] one byte over, page 2 a PDF bomb stream.
pub(crate) fn expanding_then_bomb_pdf() -> Vec<u8> {
    let map = [(1u8, letters(256)), (2u8, letters(241))];
    let map: Vec<(u8, &str)> = map.iter().map(|(c, t)| (*c, t.as_str())).collect();
    let mut w = pages_writer(vec![expanding_content(1_048_575, "02"), bomb("")], "");
    for (n, obj) in expanding_font(&map) {
        w.object(n, obj);
    }
    small(w.finish())
}

/// E2E-SEC-011: `<01>` is mapped to 128 letters `é` (256 bytes), shown 2 000 000 times.
pub(crate) fn one_code_long_text_pdf() -> Vec<u8> {
    let long = "é".repeat(128);
    page_pdf(expanding_content(1_999_999, "01"), "", expanding_font(&[(1, long.as_str())]))
}

/// E2E-006: one page whose content draws nothing.
pub(crate) fn scan_pdf() -> Vec<u8> {
    page_pdf(plain("q Q"), "", vec![])
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
