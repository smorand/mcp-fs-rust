//! pkt-line framing for the git smart HTTP protocol.
//!
//! Port of the C# `Git/Http/PktLine.cs`. A packet is a 4 digit lowercase hex
//! length (the length includes the 4 prefix bytes) followed by that many payload
//! bytes. Three lengths are special: `0000` flush, `0001` delimiter, `0002`
//! response end.
//!
//! The C# reads from a `Stream`; here the request body is already buffered by axum,
//! so [`PktReader`] walks a slice. Same semantics, including "EOF is a flush".

/// Packet classes, mirroring the C# `PacketType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    Data,
    Flush,
    Delimiter,
    ResponseEnd,
}

pub const FLUSH_BYTES: &[u8; 4] = b"0000";
pub const DELIMITER_BYTES: &[u8; 4] = b"0001";
pub const RESPONSE_END_BYTES: &[u8; 4] = b"0002";

/// The largest payload a single packet can carry (0xffff total, 4 for the prefix).
pub const MAX_PAYLOAD: usize = 0xffff - 4;

/// Encode a text line: 4 hex length prefix plus the UTF-8 bytes.
pub fn encode(line: &str) -> Vec<u8> {
    encode_raw(line.as_bytes())
}

/// Encode arbitrary bytes, used for lines carrying NUL separated capabilities and
/// for side-band chunks. Payloads longer than [`MAX_PAYLOAD`] are truncated
/// because the length would not fit in 4 hex digits; callers chunk first.
pub fn encode_raw(data: &[u8]) -> Vec<u8> {
    let data = if data.len() > MAX_PAYLOAD { &data[..MAX_PAYLOAD] } else { data };
    let total = data.len() + 4;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(format!("{total:04x}").as_bytes());
    out.extend_from_slice(data);
    out
}

/// The flush packet `0000`.
pub fn flush() -> Vec<u8> {
    FLUSH_BYTES.to_vec()
}

/// The delimiter packet `0001`.
pub fn delimiter() -> Vec<u8> {
    DELIMITER_BYTES.to_vec()
}

/// Sequential reader over a buffered pkt-line stream.
pub struct PktReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> PktReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Bytes not yet consumed. After the ref update section of a push this is the
    /// raw packfile, which is not pkt-line framed.
    pub fn remaining(&self) -> &'a [u8] {
        &self.buf[self.pos.min(self.buf.len())..]
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    /// Read one packet. Returns `(None, Flush)` at end of input, matching the C#
    /// treatment of EOF, and `(Some(&[]), Data)` for the empty packet `0004`.
    pub fn read_packet(&mut self) -> (Option<&'a [u8]>, PacketType) {
        if self.pos + 4 > self.buf.len() {
            self.pos = self.buf.len();
            return (None, PacketType::Flush);
        }
        let prefix = &self.buf[self.pos..self.pos + 4];
        self.pos += 4;
        match prefix {
            b"0000" => return (None, PacketType::Flush),
            b"0001" => return (None, PacketType::Delimiter),
            b"0002" => return (None, PacketType::ResponseEnd),
            _ => {}
        }
        let total = match std::str::from_utf8(prefix)
            .ok()
            .and_then(|s| usize::from_str_radix(s, 16).ok())
        {
            Some(n) => n,
            // A non hex prefix is unrecoverable: stop, as the C# does by
            // returning a flush.
            None => {
                self.pos = self.buf.len();
                return (None, PacketType::Flush);
            }
        };
        if total < 4 {
            self.pos = self.buf.len();
            return (None, PacketType::Flush);
        }
        let data_len = total - 4;
        if data_len == 0 {
            return (Some(&self.buf[self.pos..self.pos]), PacketType::Data);
        }
        // A truncated packet yields whatever arrived, like the C# partial read.
        let end = (self.pos + data_len).min(self.buf.len());
        let data = &self.buf[self.pos..end];
        self.pos = end;
        (Some(data), PacketType::Data)
    }

    /// [`Self::read_packet`] decoded as UTF-8 with the trailing newline trimmed.
    pub fn read_line(&mut self) -> (Option<String>, PacketType) {
        let (data, kind) = self.read_packet();
        match data {
            None => (None, kind),
            Some(d) => {
                let s = String::from_utf8_lossy(d).trim_end_matches('\n').to_string();
                (Some(s), kind)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_prefixes_the_total_length_in_hex() {
        assert_eq!(encode("a"), b"0005a");
        assert_eq!(encode("hello"), b"0009hello");
        // "# service=git-upload-pack\n" is 26 bytes, 26 + 4 = 30 = 0x1e
        assert_eq!(
            encode("# service=git-upload-pack\n"),
            b"001e# service=git-upload-pack\n".to_vec()
        );
    }

    #[test]
    fn encode_empty_payload_is_0004() {
        assert_eq!(encode(""), b"0004");
        assert_eq!(encode_raw(&[]), b"0004");
    }

    #[test]
    fn flush_and_delimiter_are_literal() {
        assert_eq!(flush(), b"0000");
        assert_eq!(delimiter(), b"0001");
        assert_eq!(FLUSH_BYTES, b"0000");
        assert_eq!(RESPONSE_END_BYTES, b"0002");
    }

    #[test]
    fn encode_raw_keeps_binary_payloads_intact() {
        let payload = b"\x00\x01\xffbin";
        let pkt = encode_raw(payload);
        assert_eq!(&pkt[..4], b"000a");
        assert_eq!(&pkt[4..], payload);
    }

    #[test]
    fn encode_raw_truncates_at_the_hex_limit() {
        let big = vec![b'x'; MAX_PAYLOAD + 100];
        let pkt = encode_raw(&big);
        assert_eq!(&pkt[..4], b"ffff");
        assert_eq!(pkt.len(), 0xffff);
    }

    #[test]
    fn round_trip_data_flush_and_empty() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&encode("want abc\n"));
        buf.extend_from_slice(&encode(""));
        buf.extend_from_slice(&flush());
        buf.extend_from_slice(&encode("after flush\n"));
        buf.extend_from_slice(&delimiter());

        let mut r = PktReader::new(&buf);
        let (d, t) = r.read_packet();
        assert_eq!(t, PacketType::Data);
        assert_eq!(d.unwrap(), b"want abc\n");

        let (d, t) = r.read_packet();
        assert_eq!(t, PacketType::Data);
        assert_eq!(d.unwrap(), b"", "the empty packet carries a zero length payload");

        let (d, t) = r.read_packet();
        assert_eq!(t, PacketType::Flush);
        assert!(d.is_none());

        let (line, t) = r.read_line();
        assert_eq!(t, PacketType::Data);
        assert_eq!(line.unwrap(), "after flush", "read_line trims the newline");

        let (d, t) = r.read_packet();
        assert_eq!(t, PacketType::Delimiter);
        assert!(d.is_none());

        // past the end: EOF is reported as a flush
        let (d, t) = r.read_packet();
        assert_eq!(t, PacketType::Flush);
        assert!(d.is_none());
    }

    #[test]
    fn read_line_handles_nul_separated_capabilities() {
        let raw = b"0000000000000000000000000000000000000000 refs/heads/main\0report-status\n";
        let buf = encode_raw(raw);
        let mut r = PktReader::new(&buf);
        let (line, t) = r.read_line();
        assert_eq!(t, PacketType::Data);
        let line = line.unwrap();
        assert!(line.contains('\0'));
        assert_eq!(line.split('\0').next().unwrap().split(' ').count(), 2);
    }

    #[test]
    fn response_end_packet_is_recognized() {
        let buf = b"0002".to_vec();
        let mut r = PktReader::new(&buf);
        assert_eq!(r.read_packet().1, PacketType::ResponseEnd);
    }

    #[test]
    fn remaining_exposes_the_unframed_tail() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&encode("cmd\n"));
        buf.extend_from_slice(&flush());
        buf.extend_from_slice(b"PACKrawbytes");

        let mut r = PktReader::new(&buf);
        assert_eq!(r.read_line().0.unwrap(), "cmd");
        assert_eq!(r.read_packet().1, PacketType::Flush);
        assert_eq!(r.remaining(), b"PACKrawbytes");
        assert_eq!(r.position(), buf.len() - 12);
    }

    #[test]
    fn malformed_prefix_stops_the_reader() {
        let buf = b"zzzzsome garbage".to_vec();
        let mut r = PktReader::new(&buf);
        assert_eq!(r.read_packet().1, PacketType::Flush);
        assert!(r.remaining().is_empty(), "reader must not loop on garbage");

        // a length below the 4 byte minimum is treated as end of stream too
        let buf2 = b"0003x".to_vec();
        let mut r2 = PktReader::new(&buf2);
        assert_eq!(r2.read_packet().1, PacketType::Flush);
    }

    #[test]
    fn truncated_packet_returns_what_arrived() {
        // claims 10 payload bytes, only 3 present
        let buf = b"000ea".to_vec();
        let mut r = PktReader::new(&buf);
        let (d, t) = r.read_packet();
        assert_eq!(t, PacketType::Data);
        assert_eq!(d.unwrap(), b"a");
        assert_eq!(r.read_packet().1, PacketType::Flush);
    }

    #[test]
    fn empty_input_is_immediately_flush() {
        let buf: Vec<u8> = Vec::new();
        let mut r = PktReader::new(&buf);
        assert_eq!(r.read_packet(), (None, PacketType::Flush));
        assert!(r.remaining().is_empty());
    }
}
