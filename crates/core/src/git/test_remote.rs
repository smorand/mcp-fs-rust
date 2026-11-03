//! A test only HTTP smart server for the tag legs a `file://` remote cannot carry
//! (SPEC-0078): it answers a chosen HTTP status, runs the real server hooks, drops
//! the answer of a push after applying it, and allows a shallow fetch, because the
//! services behind it are the real `git upload-pack` and `git receive-pack`.
//!
//! One request per connection (`Connection: close`), a loopback port picked by the
//! operating system, and the whole thing stops when the [`Remote`] is dropped.

use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

/// How the double answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Behavior {
    /// Forward every request to the real git services.
    Normal,
    /// Answer every request with this HTTP status and `Retry-After: 1`.
    Rate(u16),
    /// Apply a push through the real `git receive-pack`, then close the
    /// connection without sending the answer.
    DropAfterPush,
    /// Close the connection when a push arrives, before `receive-pack` runs.
    DropBeforePush,
}

/// A running double; the server stops when this is dropped.
pub(crate) struct Remote {
    port: u16,
    accept: tokio::task::JoinHandle<()>,
}

impl Remote {
    /// Serve the bare repository `repo_dir` on a free loopback port.
    pub(crate) async fn start(repo_dir: &Path, behavior: Behavior) -> Remote {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind a loopback port");
        let port = listener.local_addr().expect("local address").port();
        let dir = repo_dir.to_path_buf();
        let accept = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let dir = dir.clone();
                tokio::spawn(async move {
                    // A client that gives up mid request is not a test failure.
                    let _ = serve(stream, &dir, behavior).await;
                });
            }
        });
        Remote { port, accept }
    }

    /// The base URL of the repository.
    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/repo.git", self.port)
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

/// A buffered reader over a connection, enough for a request line, headers, and a
/// body that is either `Content-Length` sized or chunked.
struct Wire<'a> {
    stream: &'a mut TcpStream,
    buf: Vec<u8>,
}

impl Wire<'_> {
    async fn fill(&mut self) -> io::Result<()> {
        let mut chunk = [0u8; 8192];
        let n = self.stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    /// The bytes up to the next CRLF, consumed with it.
    async fn line(&mut self) -> io::Result<String> {
        loop {
            if let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8_lossy(&self.buf[..i]).to_string();
                self.buf.drain(..i + 2);
                return Ok(line);
            }
            self.fill().await?;
        }
    }

    async fn take(&mut self, n: usize) -> io::Result<Vec<u8>> {
        while self.buf.len() < n {
            self.fill().await?;
        }
        Ok(self.buf.drain(..n).collect())
    }
}

async fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    let mut wire = Wire { stream, buf: Vec::new() };
    let request_line = wire.line().await?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let (mut length, mut chunked) = (0usize, false);
    loop {
        let header = wire.line().await?;
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            } else if name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
                chunked = true;
            }
        }
    }
    let body = if chunked {
        let mut body = Vec::new();
        loop {
            let size_line = wire.line().await?;
            let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("0").trim(), 16)
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            if size == 0 {
                // The trailer, if any, ends with an empty line.
                while !wire.line().await?.is_empty() {}
                break;
            }
            body.extend(wire.take(size).await?);
            wire.take(2).await?;
        }
        body
    } else {
        wire.take(length).await?
    };
    Ok(Request { method, target, body })
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Status",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await
}

/// One `pkt-line` with the 4 hexadecimal digit length prefix.
fn pkt_line(text: &str) -> Vec<u8> {
    format!("{:04x}{text}", text.len() + 4).into_bytes()
}

/// Runs `git <service> --stateless-rpc [--advertise-refs] <dir>` with `input` on
/// its standard input and returns what it wrote.
async fn run_git(service: &str, dir: &Path, advertise: bool, input: &[u8]) -> io::Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.arg(service).arg("--stateless-rpc");
    if advertise {
        command.arg("--advertise-refs");
    }
    let mut child = command
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let body = input.to_vec();
    let writer = tokio::spawn(async move {
        // The child may exit before reading everything (a rejected push).
        let _ = stdin.write_all(&body).await;
    });
    let output = child.wait_with_output().await?;
    let _ = writer.await;
    Ok(output.stdout)
}

async fn serve(mut stream: TcpStream, dir: &Path, behavior: Behavior) -> io::Result<()> {
    let request = read_request(&mut stream).await?;
    if let Behavior::Rate(status) = behavior {
        return respond(&mut stream, status, "text/plain", "Retry-After: 1\r\n", b"").await;
    }
    let (path, query) = request.target.split_once('?').unwrap_or((request.target.as_str(), ""));
    let dir = dir.to_path_buf();
    match (request.method.as_str(), path) {
        ("GET", "/repo.git/info/refs") => {
            let service = match query.strip_prefix("service=") {
                Some("git-upload-pack") => "upload-pack",
                Some("git-receive-pack") => "receive-pack",
                _ => return respond(&mut stream, 404, "text/plain", "", b"").await,
            };
            let mut body = pkt_line(&format!("# service=git-{service}\n"));
            body.extend_from_slice(b"0000");
            body.extend(run_git(service, &dir, true, b"").await?);
            let content_type = format!("application/x-git-{service}-advertisement");
            respond(&mut stream, 200, &content_type, "", &body).await
        }
        ("POST", "/repo.git/git-upload-pack") => {
            let body = run_git("upload-pack", &dir, false, &request.body).await?;
            respond(&mut stream, 200, "application/x-git-upload-pack-result", "", &body).await
        }
        ("POST", "/repo.git/git-receive-pack") => {
            if behavior == Behavior::DropBeforePush {
                return Ok(());
            }
            let body = run_git("receive-pack", &dir, false, &request.body).await?;
            if behavior == Behavior::DropAfterPush {
                return Ok(());
            }
            respond(&mut stream, 200, "application/x-git-receive-pack-result", "", &body).await
        }
        _ => respond(&mut stream, 404, "text/plain", "", b"").await,
    }
}
