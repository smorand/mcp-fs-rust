//! SPEC-0093 test harness: a raw TCP stand in for a remote site, and the served
//! app with the optional web families on (DEC-008, DEC-009).
//!
//! The stand in writes hand made HTTP/1.1 so a test controls the framing and the
//! pause between the headers and the body, and reads back how many body bytes
//! reached the socket and whether the peer went away before the scripted end.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};

use crate::state::AppState;
use crate::tools::tool_surface_e2e::{app_with_atlas, setup};

/// The letter count of the spec's literal flood (spec 9.3).
pub(crate) const FLOOD_BYTES: u64 = 1_073_741_824;
const PIECE: usize = 65_536;

/// What a stand in saw.
#[derive(Default)]
pub(crate) struct Record {
    pub(crate) requests: AtomicUsize,
    pub(crate) body_sent: AtomicU64,
    pub(crate) closed_early: AtomicBool,
}

/// What a route answers.
#[derive(Clone)]
pub(crate) enum Script {
    Fixed {
        body: Vec<u8>,
        content_type: Option<String>,
        chunked: bool,
        encoding: Option<String>,
    },
    Letters {
        letter: u8,
        n: u64,
        chunked: bool,
    },
    Flood {
        endless: bool,
        encoding: Option<String>,
    },
    /// `prefix`, then `n` times `letter`, chunked.
    Prefixed {
        prefix: Vec<u8>,
        letter: u8,
        n: u64,
    },
    /// Headers declaring `length`, a pause, then the body of `then` unframed.
    Declared {
        length: u64,
        pause: Duration,
        then: Box<Script>,
    },
    /// Headers, then `per_second` letters every second for `seconds`, chunked.
    Trickle {
        per_second: u8,
        seconds: u32,
    },
    /// A 302 to `location`, no body.
    Redirect {
        location: String,
    },
    /// A failure `code` declaring `declared` body bytes it never sends.
    Status {
        code: u16,
        declared: u64,
    },
}

/// A site on `127.0.0.1:0` answering `routes` by path prefix; returns its port.
pub(crate) async fn stand_in(routes: Vec<(&'static str, Script)>) -> (u16, Arc<Record>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let record = Arc::new(Record::default());
    let rec = record.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            let routes = routes.clone();
            let rec = rec.clone();
            tokio::spawn(async move { serve_one(stream, routes, rec).await });
        }
    });
    (port, record)
}

async fn serve_one(stream: TcpStream, routes: Vec<(&'static str, Script)>, rec: Arc<Record>) {
    let (mut reader, mut writer) = stream.into_split();
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    rec.requests.fetch_add(1, Ordering::SeqCst);
    let line = String::from_utf8_lossy(&head);
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let Some((_, script)) = routes.iter().find(|(p, _)| path.starts_with(p)) else {
        let _ = writer.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
        return;
    };
    let finished = write_script(&mut writer, &mut reader, script, &rec).await;
    if !finished {
        rec.closed_early.store(true, Ordering::SeqCst);
    }
}

fn header_block(status: &str, extra: &[String]) -> Vec<u8> {
    let mut h = format!("HTTP/1.1 {status}\r\n");
    for e in extra {
        h.push_str(e);
        h.push_str("\r\n");
    }
    h.push_str("\r\n");
    h.into_bytes()
}

/// Writes the answer; false when the peer went away before the scripted end.
async fn write_script(
    w: &mut OwnedWriteHalf,
    r: &mut tokio::net::tcp::OwnedReadHalf,
    script: &Script,
    rec: &Record,
) -> bool {
    match script {
        Script::Fixed { body, content_type, chunked, encoding } => {
            let h = headers(*chunked, body.len() as u64, content_type.as_deref(), encoding);
            if w.write_all(&header_block("200 OK", &h)).await.is_err() {
                return false;
            }
            let pieces = body.chunks(PIECE).map(<[u8]>::to_vec);
            write_pieces(w, pieces, *chunked, rec).await
        }
        Script::Letters { letter, n, chunked } => {
            let h = headers(*chunked, *n, None, &None);
            if w.write_all(&header_block("200 OK", &h)).await.is_err() {
                return false;
            }
            write_pieces(w, letter_pieces(*letter, *n), *chunked, rec).await
        }
        Script::Flood { endless, encoding } => {
            let h = headers(true, 0, None, encoding);
            if w.write_all(&header_block("200 OK", &h)).await.is_err() {
                return false;
            }
            let n = if *endless { u64::MAX } else { FLOOD_BYTES };
            write_pieces(w, letter_pieces(b'a', n), true, rec).await
        }
        Script::Prefixed { prefix, letter, n } => {
            let h = headers(true, 0, None, &None);
            if w.write_all(&header_block("200 OK", &h)).await.is_err() {
                return false;
            }
            let pieces = std::iter::once(prefix.clone()).chain(letter_pieces(*letter, *n));
            write_pieces(w, pieces, true, rec).await
        }
        Script::Declared { length, pause, then } => {
            let h = headers(false, *length, None, &None);
            if w.write_all(&header_block("200 OK", &h)).await.is_err() {
                return false;
            }
            // A peer that refused on the headers closes during the pause; reading
            // returns 0 then, which is how the early close is noticed.
            let mut b = [0u8; 1];
            tokio::select! {
                _ = tokio::time::sleep(*pause) => {}
                _ = r.read(&mut b) => return false,
            }
            match then.as_ref() {
                Script::Letters { letter, n, .. } => {
                    write_pieces(w, letter_pieces(*letter, *n), false, rec).await
                }
                Script::Fixed { body, .. } => {
                    write_pieces(w, body.chunks(PIECE).map(<[u8]>::to_vec), false, rec).await
                }
                _ => true,
            }
        }
        Script::Redirect { location } => {
            let h = vec![format!("Location: {location}"), "Content-Length: 0".to_string()];
            w.write_all(&header_block("302 Found", &h)).await.is_ok()
        }
        Script::Status { code, declared } => {
            let h = headers(false, *declared, None, &None);
            w.write_all(&header_block(&format!("{code} {}", reason(*code)), &h)).await.is_ok()
        }
        Script::Trickle { per_second, seconds } => {
            let h = headers(true, 0, None, &None);
            if w.write_all(&header_block("200 OK", &h)).await.is_err() {
                return false;
            }
            for _ in 0..*seconds {
                let piece = vec![b'x'; *per_second as usize];
                if write_piece(w, &piece, true, rec).await.is_err() {
                    return false;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            w.write_all(b"0\r\n\r\n").await.is_ok()
        }
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        302 => "Found",
        500 => "Internal Server Error",
        404 => "Not Found",
        _ => "Status",
    }
}

fn headers(
    chunked: bool,
    length: u64,
    content_type: Option<&str>,
    encoding: &Option<String>,
) -> Vec<String> {
    let mut h = vec![if chunked {
        "Transfer-Encoding: chunked".to_string()
    } else {
        format!("Content-Length: {length}")
    }];
    if let Some(c) = content_type {
        h.push(format!("Content-Type: {c}"));
    }
    if let Some(e) = encoding {
        h.push(format!("Content-Encoding: {e}"));
    }
    h
}

fn letter_pieces(letter: u8, n: u64) -> impl Iterator<Item = Vec<u8>> {
    let mut left = n;
    std::iter::from_fn(move || {
        if left == 0 {
            return None;
        }
        let take = left.min(PIECE as u64);
        left -= take;
        Some(vec![letter; take as usize])
    })
}

async fn write_piece(
    w: &mut OwnedWriteHalf,
    piece: &[u8],
    chunked: bool,
    rec: &Record,
) -> std::io::Result<()> {
    if chunked {
        let mut framed = format!("{:x}\r\n", piece.len()).into_bytes();
        framed.extend_from_slice(piece);
        framed.extend_from_slice(b"\r\n");
        w.write_all(&framed).await?;
    } else {
        w.write_all(piece).await?;
    }
    rec.body_sent.fetch_add(piece.len() as u64, Ordering::SeqCst);
    Ok(())
}

async fn write_pieces(
    w: &mut OwnedWriteHalf,
    pieces: impl Iterator<Item = Vec<u8>>,
    chunked: bool,
    rec: &Record,
) -> bool {
    for p in pieces {
        if write_piece(w, &p, chunked, rec).await.is_err() {
            return false;
        }
    }
    !chunked || w.write_all(b"0\r\n\r\n").await.is_ok()
}

/// The served app with the web family (and the documentation family when a base
/// is given) on, plus `/downloads`, `/pages` and `/docs` in `atlas`.
pub(crate) async fn served(
    web: bool,
    context7_base: Option<String>,
) -> (Router, String, Arc<AppState>, TempDir) {
    served_with_base(web, context7_base, None).await
}

/// [`served`] with the search, news and suggestion hosts pointed at `test_base`.
pub(crate) async fn served_with_base(
    web: bool,
    context7_base: Option<String>,
    test_base: Option<String>,
) -> (Router, String, Arc<AppState>, TempDir) {
    let d = tempfile::tempdir().unwrap();
    let (mut c, token) = setup(d.path());
    c.web.enabled = web;
    c.web.test_base = test_base;
    if let Some(base) = context7_base {
        c.context7.enabled = true;
        c.context7.api_url = base;
    }
    let (app, state) = app_with_atlas(c).await;
    let client = state.stores.client("atlas").await.unwrap();
    for dir in ["/downloads", "/pages", "/docs"] {
        client.makedirs(dir, true).await.unwrap();
    }
    (app, token, state, d)
}

/// The member `tool_surface_e2e::setup` mints a token for.
pub(crate) const MEMBER: &str = "mia@test.com";

/// A refused write left nothing behind, read on three channels besides the answer:
/// the folder listing has no `path` (or `path` still holds `before`), the session
/// audit trail has no write line for `path`, and a 52 428 800 byte write of
/// `/docs/fill.bin` still succeeds, which it cannot if the refused call charged
/// the session allowance. The fill goes last because it spends the allowance.
pub(crate) async fn nothing_left(
    state: &AppState,
    folder: &str,
    path: &str,
    before: Option<&[u8]>,
) {
    let client = state.stores.client("atlas").await.unwrap();
    let name = path.rsplit('/').next().unwrap();
    let listing = client.list_dir(folder).await.unwrap();
    let present = listing.iter().any(|n| n.name == name);
    match before {
        None => assert!(!present, "{path} must not exist, folder holds {listing:?}"),
        Some(bytes) => assert_eq!(client.read_bytes(path).await.unwrap(), bytes, "{path} changed"),
    }
    let written = state
        .safety
        .audit(MEMBER, "atlas")
        .into_iter()
        .any(|e| e.path == path && (e.op == "write" || e.op == "append"));
    assert!(!written, "a write line was audited for {path}");
    let fill = vec![b'f'; 52_428_800];
    crate::core::fs_ops::write_bytes(
        &client,
        &state.safety,
        MEMBER,
        "atlas",
        "/docs/fill.bin",
        &fill,
        true,
        true,
    )
    .await
    .expect("the allowance is untouched, so the whole of it can still be written");
}
