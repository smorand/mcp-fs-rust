//! The external document to Markdown converter, in two interchangeable forms: a
//! CLI binary reading a file and answering on stdout, or an HTTP endpoint taking
//! `multipart/form-data`. Both are stateless: bytes in, Markdown out.
//!
//! The service is built once at boot (unlike the OCR provider, which is rebuilt
//! per call) so the HTTP client and its connection pool are reused, and so tests
//! can inject a stub without touching config.
//!
//! # Sandboxing the CLI
//!
//! The converter is an arbitrary third party binary, so containment is by
//! construction rather than by trust. Every call gets a fresh `TempDir`; the
//! input is written inside it under a sanitized single segment name; the child
//! runs with that directory as its working directory and is handed a RELATIVE
//! `./name.ext`, so a tool that writes beside its input writes inside the
//! sandbox; `TMPDIR`/`TMP`/`TEMP` point there too; the command is an argv list,
//! never a shell string; only stdout is read; stderr is captured, capped and
//! surfaced only on failure; and on timeout the child is killed and reaped
//! before the directory is removed, so no process is left writing into a
//! directory being deleted.
//!
//! This is NOT a hard OS sandbox. A converter that writes to an absolute path or
//! to `$HOME` escapes it. Real containment is the operator's call through argv[0]
//! (`sandbox-exec`, `bwrap`, `docker run`), which is why the command is a list.
//!
//! Security: the API token is a [`crate::config::Secret`]. It is read through
//! `expose()` at the single point that builds the request header and is never
//! logged, never echoed in an error and never returned to the caller.

use crate::config::{DocServiceApiConfig, DocServiceCliConfig, DocServiceConfig, doc_service_mode};
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt as _;

/// The extensions the service accepts unless `doc_service.extensions` narrows it.
///
/// PowerPoint, Word, PDF, audio and video only: `.xlsx` and images are already
/// covered by the built-in extractor behind `fs.extract_text`.
pub const DOC_SERVICE_EXTS: &[&str] = &[
    ".pptx", ".pptm", ".potx", ".ppsx", ".ppt", // PowerPoint
    ".docx", ".doc", // Word
    ".pdf", // PDF
    ".mp3", ".m4a", ".wav", ".ogg", ".flac", ".aac", ".opus", ".wma", // audio
    ".mp4", ".mov", ".mkv", ".webm", ".avi", ".m4v", ".mpeg", ".mpg", // video
];

/// How much stderr is kept from a failed converter. `doc-convert` writes
/// megabytes of progress bars there, and the TAIL is where the real cause is, so
/// the head is what gets dropped.
const STDERR_CAP: usize = 8 * 1024;

/// How much of a failing HTTP body is quoted back. The head, here: an error page
/// says what went wrong in its first line.
const BODY_CAP: usize = 2 * 1024;

/// The name of the converter's output manifest inside its bundle (DEC-006).
pub const MANIFEST_FILE: &str = "artifacts.json";
/// The Markdown a converter writes inside `{outdir}` (DEC-006).
pub const OUTDIR_MARKDOWN: &str = "document.md";

/// What one conversion returns (SPEC-0019 DEC-006): the conversion text, every
/// file the converter wrote beside it keyed by its relative `/` separated path,
/// and the optional manifest describing its pictures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversion {
    pub markdown: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub manifest: Option<Manifest>,
}

impl Conversion {
    /// A conversion that is only text, which is what a stdout converter yields.
    pub fn text(markdown: impl Into<String>) -> Self {
        Self { markdown: markdown.into(), ..Self::default() }
    }
}

/// The optional `artifacts.json` of a bundle. Absent fields mean an empty
/// caption, no failure marker and an empty page.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// Keyed by the picture reference's target, as written in the text.
    pub pictures: BTreeMap<String, PictureMeta>,
}

/// What the converter says about one picture.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PictureMeta {
    pub caption: String,
    pub caption_failed: bool,
    pub page: Option<i64>,
}

/// A stateless document to Markdown converter.
#[async_trait]
pub trait DocService: Send + Sync {
    /// Convert `bytes` (a document named `file_name`) to a [`Conversion`]
    /// bundle. Stateless.
    async fn convert(&self, bytes: &[u8], file_name: &str) -> Result<Conversion>;

    /// The Markdown of [`Self::convert`], for callers that need nothing else.
    async fn to_markdown(&self, bytes: &[u8], file_name: &str) -> Result<String> {
        Ok(self.convert(bytes, file_name).await?.markdown)
    }

    /// The extensions this service accepts, resolved once at construction from
    /// `doc_service.extensions` and never empty.
    ///
    /// It lives on the trait because the engine in `core::fs_ops` is handed a
    /// `&dyn DocService` and no config, yet must refuse an ineligible upload
    /// before writing a byte and must name the accepted set in the error.
    fn accepted_extensions(&self) -> &[String];

    /// Refuse to convert an input larger than this (`doc_service.max_input_bytes`).
    fn max_input_bytes(&self) -> u64;
}

/// Extension gate against the built-in set, case insensitive.
pub fn eligible(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    DOC_SERVICE_EXTS.iter().any(|e| lower.ends_with(e))
}

/// Extension gate honouring a deployment override. An empty list means the
/// built-in set, which is exactly what `doc_service.extensions: []` says.
pub fn eligible_in(path: &str, extensions: &[String]) -> bool {
    if extensions.is_empty() {
        return eligible(path);
    }
    let lower = path.to_ascii_lowercase();
    extensions.iter().any(|e| {
        let e = e.trim().to_ascii_lowercase();
        !e.is_empty() && lower.ends_with(&e)
    })
}

/// Build the service the config names, or `None` when the feature is off.
///
/// An unknown mode is rejected here as well as at boot validation, because a
/// service can be built from a config that never went through [`crate::config::ServerConfig::validate`].
pub fn from_config(cfg: &DocServiceConfig) -> Result<Option<Arc<dyn DocService>>> {
    if !cfg.enabled {
        return Ok(None);
    }
    let extensions = resolve_extensions(&cfg.extensions);
    match cfg.mode.as_str() {
        doc_service_mode::CLI => {
            Ok(Some(Arc::new(CliDocService::new(cfg.cli.clone(), extensions, cfg.max_input_bytes))))
        }
        doc_service_mode::API => {
            Ok(Some(Arc::new(ApiDocService::new(&cfg.api, extensions, cfg.max_input_bytes)?)))
        }
        other => Err(ToolError::invalid_argument(format!(
            "unknown doc_service.mode '{other}', expected one of {}",
            doc_service_mode::ALL.join(", ")
        ))),
    }
}

/// The configured override, normalized, or the built-in set when it is empty.
fn resolve_extensions(configured: &[String]) -> Vec<String> {
    let narrowed: Vec<String> = configured
        .iter()
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .map(|e| if e.starts_with('.') { e } else { format!(".{e}") })
        .collect();
    if narrowed.is_empty() {
        DOC_SERVICE_EXTS.iter().map(|e| (*e).to_string()).collect()
    } else {
        narrowed
    }
}

/// A converter run as a child process in a per call temporary directory.
pub struct CliDocService {
    config: DocServiceCliConfig,
    extensions: Vec<String>,
    max_input_bytes: u64,
}

impl CliDocService {
    pub fn new(config: DocServiceCliConfig, extensions: Vec<String>, max_input_bytes: u64) -> Self {
        Self { config, extensions, max_input_bytes }
    }
}

#[async_trait]
impl DocService for CliDocService {
    #[tracing::instrument(skip(self, bytes), fields(doc.mode = "cli", doc.file = file_name))]
    async fn convert(&self, bytes: &[u8], file_name: &str) -> Result<Conversion> {
        // Rule 1: a fresh directory per call, removed on every exit path because
        // it is dropped when this function returns.
        let dir = tempfile::TempDir::new().map_err(|e| {
            ToolError::internal(format!(
                "document service: cannot create a temporary directory: {e}"
            ))
        })?;
        // Rule 2: the input lands inside it, under a single segment name keeping
        // the original extension (the converter dispatches on it).
        let name = sanitize_file_name(file_name);
        tokio::fs::write(dir.path().join(&name), bytes).await.map_err(|e| {
            ToolError::internal(format!("document service: cannot stage the input document: {e}"))
        })?;

        // With `{outdir}` the converter writes a bundle into a fresh empty
        // directory inside the sandbox (DEC-006). A sanitized input name never
        // starts with a dot, so it cannot collide with this one.
        let outdir = self
            .config
            .command
            .iter()
            .any(|a| a.contains(crate::config::OUTDIR_PLACEHOLDER))
            .then(|| dir.path().join(".bundle"));
        if let Some(out) = &outdir {
            tokio::fs::create_dir(out).await.map_err(|e| {
                ToolError::internal(format!(
                    "document service: cannot create the output directory: {e}"
                ))
            })?;
        }

        // Rules 3 and 5: an argv list, never a shell string, with `{document}`
        // expanded to a RELATIVE name so a tool writing beside its input stays in
        // the sandbox.
        let out_arg = outdir.as_ref().map(|o| o.display().to_string()).unwrap_or_default();
        let argv = expand_argv(&self.config.command, &format!("./{name}"), &out_arg);
        let Some((program, args)) = argv.split_first() else {
            return Err(ToolError::internal("doc_service.cli.command is empty"));
        };

        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .current_dir(dir.path())
            // Rule 4: temporary files go to the sandbox too. `env_clear` is NOT
            // used: a converter legitimately needs HOME, PATH and its model cache.
            .env("TMPDIR", dir.path())
            .env("TMP", dir.path())
            .env("TEMP", dir.path())
            .stdin(Stdio::null())
            // Rule 6: the result is stdout and only stdout, and stderr is captured
            // so a progress bar never reaches a log line.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|e| {
            ToolError::internal(format!("document service: cannot start '{program}': {e}"))
        })?;
        let mut child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::internal("document service: stdout was not captured"))?;
        let mut child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::internal("document service: stderr was not captured"))?;

        // The pipes are drained concurrently with the wait: a converter that fills
        // its stderr buffer would otherwise block forever on a write nobody reads.
        let waited = tokio::time::timeout(Duration::from_secs(self.config.timeout_secs), async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let (read_out, read_err) = tokio::join!(
                child_stdout.read_to_end(&mut out),
                child_stderr.read_to_end(&mut err)
            );
            read_out?;
            read_err?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((status, out, err))
        })
        .await;

        let (status, stdout, stderr) = match waited {
            Ok(Ok(triple)) => triple,
            Ok(Err(e)) => {
                return Err(ToolError::internal(format!("document service I/O error: {e}")));
            }
            Err(_) => {
                // Rule 7: kill and reap BEFORE `dir` is dropped at the return below,
                // so no process is left writing into a directory being deleted.
                let _ = child.kill().await;
                return Err(ToolError::internal(format!(
                    "doc service timed out after {}s",
                    self.config.timeout_secs
                )));
            }
        };

        let tail = tail_of(&stderr, STDERR_CAP);
        if !status.success() {
            return Err(ToolError::internal(format!(
                "document service exited with {}: {tail}",
                exit_label(&status)
            )));
        }
        let conversion = match outdir {
            Some(out) => read_bundle(out, self.max_input_bytes).await?,
            None => Conversion::text(String::from_utf8_lossy(&stdout).to_string()),
        };
        if conversion.markdown.trim().is_empty() {
            return Err(ToolError::internal(format!(
                "document service exited with {} but produced no output: {tail}",
                exit_label(&status)
            )));
        }
        Ok(conversion)
    }

    fn accepted_extensions(&self) -> &[String] {
        &self.extensions
    }

    fn max_input_bytes(&self) -> u64 {
        self.max_input_bytes
    }
}

/// A converter behind an HTTP endpoint taking `multipart/form-data`.
pub struct ApiDocService {
    config: DocServiceApiConfig,
    extensions: Vec<String>,
    max_input_bytes: u64,
    http: reqwest::Client,
}

impl ApiDocService {
    /// One client for the whole process, so the connection pool is reused and the
    /// configured timeout is enforced by the transport rather than by a race.
    pub fn new(
        config: &DocServiceApiConfig,
        extensions: Vec<String>,
        max_input_bytes: u64,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("mcp-fs-doc-service/1.0")
            .build()
            .map_err(|e| ToolError::internal(format!("document service http client error: {e}")))?;
        Ok(Self { config: config.clone(), extensions, max_input_bytes, http })
    }
}

#[async_trait]
impl DocService for ApiDocService {
    #[tracing::instrument(skip(self, bytes), fields(doc.mode = "api", doc.file = file_name))]
    async fn convert(&self, bytes: &[u8], file_name: &str) -> Result<Conversion> {
        let mime = crate::docs::mime::guess(file_name).unwrap_or("application/octet-stream");
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(file_name.to_string())
            .mime_str(mime)
            .map_err(|e| {
                ToolError::internal(format!("document service: invalid content type '{mime}': {e}"))
            })?;
        // The part name belongs to the endpoint we call, not to us, so it comes
        // from the config (`file` by default, which is what our own fake service
        // and most converters expect).
        let form =
            reqwest::multipart::Form::new().part(self.config.file_field.trim().to_string(), part);

        let mut request = self.http.post(self.config.url.trim()).multipart(form);
        // The token is sent verbatim, which is how one setting covers both a bearer
        // scheme and a bare key. An empty token means the endpoint is unauthenticated,
        // and sending an empty header would look like a failed authentication.
        if !self.config.auth_token.is_empty() {
            request =
                request.header(self.config.auth_header.trim(), self.config.auth_token.expose());
        }

        let response = request.send().await.map_err(|e| {
            ToolError::internal(format!("document service request failed: {}", transport_kind(&e)))
        })?;
        let status = response.status();
        let is_json = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim_start().starts_with("application/json"));
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ToolError::internal(format!(
                "document service returned HTTP {}: {}",
                status.as_u16(),
                head_of(&body, BODY_CAP)
            )));
        }

        // A configured `response_field` keeps its meaning; otherwise a JSON
        // answer is a bundle (DEC-006) and anything else is the Markdown body.
        let field = self.config.response_field.trim();
        if field.is_empty() {
            return if is_json {
                parse_api_bundle(&body, self.max_input_bytes)
            } else {
                Ok(Conversion::text(body))
            };
        }
        let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            ToolError::internal(format!(
                "document service returned a non JSON body while \
                 doc_service.api.response_field is set: {e}"
            ))
        })?;
        parsed.get(field).and_then(serde_json::Value::as_str).map(Conversion::text).ok_or_else(
            || {
                ToolError::internal(format!(
                    "document service response carries no string field '{field}'"
                ))
            },
        )
    }

    fn accepted_extensions(&self) -> &[String] {
        &self.extensions
    }

    fn max_input_bytes(&self) -> u64 {
        self.max_input_bytes
    }
}

/// Reduce an arbitrary file name to one safe path segment, keeping the extension.
///
/// The converter must never be handed a name that can climb out of the sandbox,
/// and a leading dot would produce a hidden file the tool may then ignore.
fn sanitize_file_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let (raw_stem, raw_ext) = match base.rfind('.') {
        Some(i) if i > 0 => (&base[..i], &base[i..]),
        _ => (base, ""),
    };
    let keep = |s: &str| -> String {
        s.chars()
            .map(
                |c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' },
            )
            .collect()
    };
    let stem = keep(raw_stem).trim_matches('.').to_string();
    let stem = if stem.is_empty() { "document".to_string() } else { stem };
    // An "extension" made only of dots came from `..` or `.`, never from a suffix.
    let ext = keep(raw_ext);
    let ext = if ext.chars().all(|c| c == '.') { String::new() } else { ext };
    format!("{stem}{ext}")
}

/// Expand `{document}` and `{outdir}` in every argument. Config validation
/// guarantees exactly one `{document}` and at most one `{outdir}`, but a
/// service built without it must still behave.
fn expand_argv(command: &[String], document: &str, outdir: &str) -> Vec<String> {
    command
        .iter()
        .map(|a| {
            a.replace(crate::config::DOC_PLACEHOLDER, document)
                .replace(crate::config::OUTDIR_PLACEHOLDER, outdir)
        })
        .collect()
}

/// The bundle a converter wrote into `out`: `document.md` as the Markdown,
/// every regular file under `out` by its relative `/` separated path (symlinks
/// are never followed), and `artifacts.json` as the manifest. The files are
/// capped at `cap` bytes in total, like the input.
async fn read_bundle(out: std::path::PathBuf, cap: u64) -> Result<Conversion> {
    tokio::task::spawn_blocking(move || read_bundle_blocking(&out, cap))
        .await
        .map_err(|e| ToolError::internal(format!("document service: bundle read failed: {e}")))?
}

fn read_bundle_blocking(out: &std::path::Path, cap: u64) -> Result<Conversion> {
    let io = |e: std::io::Error| {
        ToolError::internal(format!("document service: cannot read its output: {e}"))
    };
    // `read_dir` follows a symlink, so a converter that swapped its output
    // folder for a link would make us read outside the sandbox.
    let root = std::fs::symlink_metadata(out).map_err(io)?;
    if !root.file_type().is_dir() {
        return Err(ToolError::internal(
            "document service output folder is not a real directory, refused",
        ));
    }
    let mut files = BTreeMap::new();
    let mut total: u64 = 0;
    let mut pending = vec![out.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(io)? {
            let entry = entry.map_err(io)?;
            // `file_type` does not follow a symlink, so a link is skipped here.
            let kind = entry.file_type().map_err(io)?;
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(out) else { continue };
            let key: Vec<String> = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            // Checked on the length before reading, so an oversized file is
            // never pulled into memory.
            let len = entry.metadata().map_err(io)?.len();
            if len > cap - total {
                return Err(ToolError::internal(format!(
                    "document service output is over the doc_service.max_input_bytes limit of {cap}"
                )));
            }
            let bytes = std::fs::read(&path).map_err(io)?;
            // The file may have grown since its length was taken.
            total += bytes.len() as u64;
            if total > cap {
                return Err(ToolError::internal(format!(
                    "document service output is over the doc_service.max_input_bytes limit of {cap}"
                )));
            }
            files.insert(key.join("/"), bytes);
        }
    }
    let markdown = files.get(OUTDIR_MARKDOWN).ok_or_else(|| {
        ToolError::internal(format!("document service wrote no {OUTDIR_MARKDOWN} in its output"))
    })?;
    let markdown = String::from_utf8_lossy(markdown).into_owned();
    let manifest = files.get(MANIFEST_FILE).and_then(|raw| manifest_from(raw));
    Ok(Conversion { markdown, files, manifest })
}

/// A manifest that does not parse is ignored, as if absent: captions and pages
/// are lost, the conversion is not.
fn manifest_from(raw: &[u8]) -> Option<Manifest> {
    match serde_json::from_slice(raw) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(error = %e, "document service manifest ignored");
            None
        }
    }
}

/// The JSON bundle of an api mode answer:
/// `{"markdown": "...", "files": {"<path>": "<base64>"}, "manifest": {...}}`.
fn parse_api_bundle(body: &str, cap: u64) -> Result<Conversion> {
    #[derive(Deserialize)]
    struct Bundle {
        markdown: String,
        #[serde(default)]
        files: BTreeMap<String, String>,
        #[serde(default)]
        manifest: Option<serde_json::Value>,
    }
    use base64::Engine as _;
    let bundle: Bundle = serde_json::from_str(body).map_err(|e| {
        ToolError::internal(format!("document service returned a malformed bundle: {e}"))
    })?;
    let mut files = BTreeMap::new();
    let mut total: u64 = 0;
    for (key, encoded) in bundle.files {
        let bytes =
            base64::engine::general_purpose::STANDARD.decode(encoded.trim()).map_err(|e| {
                ToolError::internal(format!(
                    "document service bundle file '{key}' is not base64: {e}"
                ))
            })?;
        total += bytes.len() as u64;
        if total > cap {
            return Err(ToolError::internal(format!(
                "document service output is over the doc_service.max_input_bytes limit of {cap}"
            )));
        }
        files.insert(key, bytes);
    }
    let manifest = bundle.manifest.and_then(|v| {
        serde_json::from_value(v)
            .map_err(|e| tracing::warn!(error = %e, "document service manifest ignored"))
            .ok()
    });
    Ok(Conversion { markdown: bundle.markdown, files, manifest })
}

/// The last `cap` bytes, decoded lossily. Byte slicing is safe here because
/// `from_utf8_lossy` replaces whatever partial character the cut produced.
fn tail_of(bytes: &[u8], cap: usize) -> String {
    let start = bytes.len().saturating_sub(cap);
    String::from_utf8_lossy(&bytes[start..]).trim().to_string()
}

/// The first `cap` characters, on a character boundary.
fn head_of(text: &str, cap: usize) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(cap) {
        Some((i, _)) => trimmed[..i].to_string(),
        None => trimmed.to_string(),
    }
}

/// How a child ended, for the failure message.
fn exit_label(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "a signal".to_string(),
    }
}

/// Classify a transport failure without quoting the request, which carries the token.
fn transport_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connection refused"
    } else if e.is_decode() {
        "malformed response"
    } else {
        "transport error"
    }
}

/// A canned service for tests in this crate: it either returns a fixed Markdown
/// or a fixed error, without touching a process or the network.
#[cfg(test)]
pub struct StubDocService {
    outcome: Result<Conversion>,
    extensions: Vec<String>,
    max_input_bytes: u64,
}

#[cfg(test)]
impl StubDocService {
    /// A service that always converts to `markdown`.
    pub fn ok(markdown: impl Into<String>) -> Self {
        Self {
            outcome: Ok(Conversion::text(markdown)),
            extensions: resolve_extensions(&[]),
            max_input_bytes: u64::MAX,
        }
    }

    /// A service that always fails, for the "the upload is never rolled back" path.
    pub fn failing(error: ToolError) -> Self {
        Self { outcome: Err(error), extensions: resolve_extensions(&[]), max_input_bytes: u64::MAX }
    }

    /// A service that always returns `conversion`, files and manifest included.
    pub fn bundle(conversion: Conversion) -> Self {
        Self {
            outcome: Ok(conversion),
            extensions: resolve_extensions(&[]),
            max_input_bytes: u64::MAX,
        }
    }

    /// Narrow the accepted extensions, as `doc_service.extensions` does.
    #[must_use]
    pub fn with_extensions(mut self, extensions: &[&str]) -> Self {
        let configured: Vec<String> = extensions.iter().map(|e| (*e).to_string()).collect();
        self.extensions = resolve_extensions(&configured);
        self
    }

    /// Narrow the input size cap, for the pre-write validation tests.
    #[must_use]
    pub fn with_max_input_bytes(mut self, max_input_bytes: u64) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }
}

#[cfg(test)]
#[async_trait]
impl DocService for StubDocService {
    async fn convert(&self, _bytes: &[u8], _file_name: &str) -> Result<Conversion> {
        self.outcome.clone()
    }

    fn accepted_extensions(&self) -> &[String] {
        &self.extensions
    }

    fn max_input_bytes(&self) -> u64 {
        self.max_input_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    // ── the extension gate ───────────────────────────────────────────────────

    #[test]
    fn eligible_is_case_insensitive_over_the_built_in_set() {
        assert!(eligible("/deck.pptx"));
        assert!(eligible("/DECK.PPTX"));
        assert!(eligible("/a/b/report.PdF"));
        assert!(eligible("/talk.mp4"));
        assert!(!eligible("/notes.txt"));
        assert!(!eligible("/sheet.xlsx"));
        assert!(!eligible("/logo.png"));
        assert!(!eligible("/noext"));
    }

    #[test]
    fn a_configured_list_narrows_the_gate_and_an_empty_one_does_not() {
        let narrowed = resolve_extensions(&["PDF".into(), " .docx ".into()]);
        assert_eq!(narrowed, vec![".pdf", ".docx"]);
        assert!(eligible_in("/a.pdf", &narrowed));
        assert!(!eligible_in("/a.pptx", &narrowed), "the override must really narrow");
        assert!(eligible_in("/a.pptx", &[]), "empty means the built-in set");
    }

    #[test]
    fn from_config_is_none_when_disabled_and_rejects_an_unknown_mode() {
        assert!(from_config(&DocServiceConfig::default()).unwrap().is_none());

        let mut cfg = DocServiceConfig { enabled: true, ..Default::default() };
        let cli = from_config(&cfg).unwrap().expect("cli mode builds");
        assert_eq!(cli.max_input_bytes(), 536_870_912);
        assert!(cli.accepted_extensions().contains(&".pdf".to_string()));

        cfg.mode = "grpc".into();
        // `Arc<dyn DocService>` has no Debug, so match rather than unwrap_err.
        let Err(e) = from_config(&cfg) else { panic!("grpc is not a doc service mode") };
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("doc_service.mode"), "{}", e.message);
    }

    #[test]
    fn a_file_name_is_reduced_to_one_safe_segment_keeping_its_extension() {
        assert_eq!(sanitize_file_name("toto.pptx"), "toto.pptx");
        assert_eq!(sanitize_file_name("/a/b/deck.pptx"), "deck.pptx");
        assert_eq!(sanitize_file_name("../../etc/passwd.pdf"), "passwd.pdf");
        assert_eq!(sanitize_file_name("my deck (final).pdf"), "my_deck__final_.pdf");
        assert_eq!(sanitize_file_name(".."), "document");
        assert_eq!(sanitize_file_name(".hidden.pdf"), "hidden.pdf");
    }

    // ── cli mode, real child processes ───────────────────────────────────────

    /// Write an executable `/bin/sh` script into `dir` and return its path. The
    /// script is built by the test rather than committed, so there is no fixture
    /// to keep in sync with it.
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        f.write_all(body.as_bytes()).unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn cli_service(argv: Vec<String>, timeout_secs: u64) -> CliDocService {
        CliDocService::new(
            DocServiceCliConfig { command: argv, timeout_secs },
            resolve_extensions(&[]),
            u64::MAX,
        )
    }

    #[tokio::test]
    async fn cli_returns_the_markdown_the_child_wrote_on_stdout() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(home.path(), "conv.sh", "cat \"$1\"\n");
        let svc =
            cli_service(vec![sh.display().to_string(), crate::config::DOC_PLACEHOLDER.into()], 30);

        let md = svc.to_markdown(b"# from the fixture\n", "deck.pptx").await.unwrap();
        assert_eq!(md, "# from the fixture\n");
    }

    /// The sandbox and the cleanup, both proven from one run: the child reports the
    /// directory it ran in and what it found there, and that directory is gone once
    /// the call returns.
    #[tokio::test]
    async fn cli_confines_the_child_to_a_tempdir_that_is_then_removed() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(
            home.path(),
            "junk.sh",
            "echo sandbox-marker > ./junk.txt\n\
             echo '# converted'\n\
             echo \"CWD=$(pwd)\"\n\
             echo \"FILES=$(ls | tr '\\n' ' ')\"\n\
             echo \"TMPDIR=$TMPDIR\"\n",
        );
        let svc =
            cli_service(vec![sh.display().to_string(), crate::config::DOC_PLACEHOLDER.into()], 30);

        let before = std::env::current_dir().unwrap();
        let md = svc.to_markdown(b"body", "deck.pptx").await.unwrap();

        let line = |key: &str| -> String {
            md.lines()
                .find_map(|l| l.strip_prefix(key))
                .unwrap_or_else(|| panic!("{key} missing from {md}"))
                .to_string()
        };
        // The child reports a resolved path (on macOS /var is a symlink to
        // /private/var), so both sides are canonicalized before comparison.
        let cwd = PathBuf::from(line("CWD="));
        let files = line("FILES=");

        // The junk landed next to the input, which is inside the sandbox.
        assert!(files.contains("junk.txt"), "the junk file should be in the cwd: {files}");
        assert!(files.contains("deck.pptx"), "the input should be in the cwd: {files}");
        let tmpdir = std::fs::canonicalize(line("TMPDIR=")).unwrap_or_else(|_| cwd.clone());
        assert_eq!(tmpdir, cwd, "TMPDIR must point at the sandbox");
        assert_ne!(cwd, before, "the child must not run in the server's cwd");
        assert!(!cwd.exists(), "the tempdir must be gone once the call returns: {cwd:?}");
        assert!(!before.join("junk.txt").exists(), "nothing may be written next to the server");
    }

    /// A converter that never finishes must not hold a task forever, must not
    /// survive the call, and must not leave its directory behind.
    #[tokio::test]
    async fn cli_timeout_kills_the_child_and_removes_the_tempdir() {
        let home = tempfile::tempdir().unwrap();
        let marker = home.path().join("marker.txt");
        let sh = script(
            home.path(),
            "slow.sh",
            "pwd > \"$1\"\n\
             sleep 30\n\
             echo survived >> \"$1\"\n",
        );
        let svc = cli_service(
            vec![
                sh.display().to_string(),
                marker.display().to_string(),
                crate::config::DOC_PLACEHOLDER.into(),
            ],
            // Long enough that a loaded machine still gets the child started, short
            // enough that the test does not become a wait.
            3,
        );

        let e = svc.to_markdown(b"body", "deck.pptx").await.unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("timed out after 3s"), "{}", e.message);

        let sandbox = PathBuf::from(std::fs::read_to_string(&marker).unwrap().trim());
        assert!(!sandbox.exists(), "the tempdir must be gone after a timeout: {sandbox:?}");

        // Past the sleep the child would have appended; a reaped child cannot.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let after = std::fs::read_to_string(&marker).unwrap();
        assert!(!after.contains("survived"), "the child was not killed: {after}");
    }

    #[tokio::test]
    async fn cli_failure_carries_the_exit_code_and_only_the_stderr_tail() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(
            home.path(),
            "noisy.sh",
            "i=0\n\
             while [ $i -lt 2000 ]; do echo \"noise line $i padding padding\" >&2; i=$((i+1)); done\n\
             echo 'the real cause' >&2\n\
             exit 3\n",
        );
        let svc =
            cli_service(vec![sh.display().to_string(), crate::config::DOC_PLACEHOLDER.into()], 30);

        let e = svc.to_markdown(b"body", "deck.pptx").await.unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("exit code 3"), "{}", e.message);
        assert!(e.message.contains("the real cause"), "the tail must survive: {}", e.message);
        assert!(!e.message.contains("noise line 0 "), "the head must be dropped");
        assert!(e.message.len() <= STDERR_CAP + 128, "stderr is capped: {} bytes", e.message.len());
    }

    #[tokio::test]
    async fn cli_empty_stdout_is_a_failure_not_an_empty_document() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(home.path(), "silent.sh", "exit 0\n");
        let svc =
            cli_service(vec![sh.display().to_string(), crate::config::DOC_PLACEHOLDER.into()], 30);

        let e = svc.to_markdown(b"body", "deck.pptx").await.unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("no output"), "{}", e.message);
    }

    /// SPEC-0019 DEC-006: with `{outdir}` the bundle is read from a fresh
    /// directory inside the sandbox: `document.md`, every regular file by its
    /// relative path, and `artifacts.json` as the manifest.
    #[tokio::test]
    async fn cli_outdir_mode_reads_the_bundle_the_converter_wrote() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(
            home.path(),
            "bundle.sh",
            "out=\"$1\"\n\
             test -z \"$(ls -A \"$out\")\" || exit 9\n\
             mkdir -p \"$out/figures\"\n\
             printf '# doc\\n![c](figures/f1.png)\\n' > \"$out/document.md\"\n\
             printf 'PNG' > \"$out/figures/f1.png\"\n\
             printf '{\"pictures\":{\"figures/f1.png\":{\"caption\":\"Chart\",\"page\":3}}}' > \"$out/artifacts.json\"\n\
             ln -s /etc/hosts \"$out/escape.txt\"\n\
             echo 'stdout is ignored'\n",
        );
        let svc = cli_service(
            vec![
                sh.display().to_string(),
                crate::config::OUTDIR_PLACEHOLDER.into(),
                crate::config::DOC_PLACEHOLDER.into(),
            ],
            30,
        );

        let c = svc.convert(b"body", "deck.pdf").await.unwrap();
        assert_eq!(c.markdown, "# doc\n![c](figures/f1.png)\n");
        assert_eq!(c.files.get("figures/f1.png").map(Vec::as_slice), Some(&b"PNG"[..]));
        assert!(!c.files.contains_key("escape.txt"), "a symlink is never followed");
        let meta = &c.manifest.expect("the manifest").pictures["figures/f1.png"];
        assert_eq!(
            meta,
            &PictureMeta { caption: "Chart".into(), caption_failed: false, page: Some(3) }
        );
        // `to_markdown` stays the bundle's Markdown.
        assert_eq!(svc.to_markdown(b"body", "deck.pdf").await.unwrap(), c.markdown);
    }

    #[tokio::test]
    async fn cli_outdir_mode_without_document_md_is_a_failure() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(home.path(), "nodoc.sh", "echo '# on stdout'\n");
        let svc = cli_service(
            vec![
                sh.display().to_string(),
                crate::config::OUTDIR_PLACEHOLDER.into(),
                crate::config::DOC_PLACEHOLDER.into(),
            ],
            30,
        );
        let e = svc.convert(b"body", "deck.pdf").await.unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("document.md"), "{}", e.message);
    }

    /// The bundle is capped by `max_input_bytes`, like the input.
    #[tokio::test]
    async fn cli_outdir_mode_caps_the_bundle_size() {
        let home = tempfile::tempdir().unwrap();
        let sh = script(
            home.path(),
            "big.sh",
            "printf '# doc' > \"$1/document.md\"\nprintf '0123456789' > \"$1/big.bin\"\n",
        );
        let svc = CliDocService::new(
            DocServiceCliConfig {
                command: vec![
                    sh.display().to_string(),
                    crate::config::OUTDIR_PLACEHOLDER.into(),
                    crate::config::DOC_PLACEHOLDER.into(),
                ],
                timeout_secs: 30,
            },
            resolve_extensions(&[]),
            8,
        );
        let e = svc.convert(b"body", "deck.pdf").await.unwrap_err();
        assert!(e.message.contains("max_input_bytes"), "{}", e.message);
    }

    /// A converter that swaps its output folder for a symlink to a directory
    /// outside the sandbox is refused: nothing behind the link is read.
    #[test]
    fn bundle_outdir_replaced_by_a_symlink_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join(OUTDIR_MARKDOWN), b"# secret").unwrap();
        std::fs::write(outside.path().join("secret.bin"), b"SECRET").unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        let out = sandbox.path().join(".bundle");
        std::os::unix::fs::symlink(outside.path(), &out).unwrap();

        let e = read_bundle_blocking(&out, 1024).unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("not a real directory"), "{}", e.message);
        assert!(!e.message.contains("secret"), "{}", e.message);
    }

    /// The size cap is checked on the entry length before any byte is read: an
    /// oversized unreadable file is refused on its size, never opened.
    #[test]
    fn bundle_size_cap_is_checked_before_reading() {
        let sandbox = tempfile::tempdir().unwrap();
        let out = sandbox.path().join(".bundle");
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join(OUTDIR_MARKDOWN), b"# doc").unwrap();
        let big = out.join("big.bin");
        std::fs::write(&big, vec![0u8; 64]).unwrap();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o000)).unwrap();

        let e = read_bundle_blocking(&out, 16).unwrap_err();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(e.message.contains("max_input_bytes"), "{}", e.message);
    }

    // ── api mode, in process stub server ─────────────────────────────────────

    /// What the stub server saw, so the request itself can be asserted.
    #[derive(Debug, Default, Clone)]
    struct Seen {
        header: Option<String>,
        part_name: String,
        file_name: String,
        content_type: String,
        body: Vec<u8>,
    }

    /// Serve one canned response on `POST /convert` and record the request.
    async fn stub_server(
        status: axum::http::StatusCode,
        body: &'static str,
        header_name: &'static str,
    ) -> (String, Arc<std::sync::Mutex<Vec<Seen>>>) {
        use axum::extract::Multipart;
        use axum::http::HeaderMap;
        use axum::routing::post;

        let seen = Arc::new(std::sync::Mutex::new(Vec::<Seen>::new()));
        let captured = seen.clone();
        let handler = move |headers: HeaderMap, mut form: Multipart| {
            let captured = captured.clone();
            async move {
                let mut record = Seen {
                    header: headers
                        .get(header_name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    ..Default::default()
                };
                while let Ok(Some(field)) = form.next_field().await {
                    record.part_name = field.name().unwrap_or_default().to_string();
                    record.file_name = field.file_name().unwrap_or_default().to_string();
                    record.content_type = field.content_type().unwrap_or_default().to_string();
                    record.body = field.bytes().await.unwrap_or_default().to_vec();
                }
                captured.lock().unwrap().push(record);
                (status, body)
            }
        };

        let app = axum::Router::new().route("/convert", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/convert"), seen)
    }

    fn api_service(cfg: DocServiceApiConfig) -> ApiDocService {
        ApiDocService::new(&cfg, resolve_extensions(&[]), u64::MAX).unwrap()
    }

    #[tokio::test]
    async fn api_sends_the_configured_header_and_a_named_file_part() {
        let (url, seen) = stub_server(axum::http::StatusCode::OK, "# ok", "x-convert-key").await;
        let svc = api_service(DocServiceApiConfig {
            url,
            auth_header: "X-Convert-Key".into(),
            auth_token: crate::config::Secret::new("Bearer t0ken"),
            ..Default::default()
        });

        svc.to_markdown(b"PK-bytes", "deck.pptx").await.unwrap();

        let seen = seen.lock().unwrap();
        let r = seen.first().expect("the stub saw a request");
        assert_eq!(r.header.as_deref(), Some("Bearer t0ken"), "the token is sent verbatim");
        assert_eq!(r.part_name, "file", "the default part name");
        assert_eq!(r.file_name, "deck.pptx", "the original name reaches the converter");
        assert_eq!(
            r.content_type,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
        assert_eq!(r.body, b"PK-bytes");
    }

    #[tokio::test]
    async fn api_names_the_file_part_the_way_the_endpoint_wants_it() {
        // A third party endpoint names the part whatever it likes, so the name is
        // configuration and not a constant of ours.
        let (url, seen) = stub_server(axum::http::StatusCode::OK, "# ok", "authorization").await;
        let svc = api_service(DocServiceApiConfig {
            url,
            file_field: "document".into(),
            ..Default::default()
        });

        svc.to_markdown(b"x", "a.pdf").await.unwrap();
        assert_eq!(seen.lock().unwrap()[0].part_name, "document");
    }

    #[tokio::test]
    async fn api_sends_no_header_when_no_token_is_configured() {
        let (url, seen) = stub_server(axum::http::StatusCode::OK, "# ok", "authorization").await;
        let svc = api_service(DocServiceApiConfig { url, ..Default::default() });

        svc.to_markdown(b"x", "a.pdf").await.unwrap();
        assert_eq!(seen.lock().unwrap()[0].header, None);
    }

    #[tokio::test]
    async fn api_reads_the_body_or_the_configured_json_field() {
        let (url, _) =
            stub_server(axum::http::StatusCode::OK, "# raw markdown", "authorization").await;
        let raw = api_service(DocServiceApiConfig { url: url.clone(), ..Default::default() });
        assert_eq!(raw.to_markdown(b"x", "a.pdf").await.unwrap(), "# raw markdown");

        let (url, _) = stub_server(
            axum::http::StatusCode::OK,
            r##"{"markdown":"# from the field","other":1}"##,
            "authorization",
        )
        .await;
        let field = api_service(DocServiceApiConfig {
            url,
            response_field: "markdown".into(),
            ..Default::default()
        });
        assert_eq!(field.to_markdown(b"x", "a.pdf").await.unwrap(), "# from the field");
    }

    /// SPEC-0019 DEC-006: an `application/json` answer is a bundle.
    #[tokio::test]
    async fn api_json_answer_is_a_bundle() {
        use axum::routing::post;
        let body = r##"{"markdown":"# b\n![x](f/a.png)\n","files":{"f/a.png":"UE5H"},"manifest":{"pictures":{"f/a.png":{"caption_failed":true}}}}"##;
        let app = axum::Router::new().route(
            "/convert",
            post(move || async move { ([("content-type", "application/json")], body) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let svc = api_service(DocServiceApiConfig {
            url: format!("http://{addr}/convert"),
            ..Default::default()
        });
        let c = svc.convert(b"x", "a.pdf").await.unwrap();
        assert_eq!(c.markdown, "# b\n![x](f/a.png)\n");
        assert_eq!(c.files.get("f/a.png").map(Vec::as_slice), Some(&b"PNG"[..]));
        let meta = &c.manifest.unwrap().pictures["f/a.png"];
        assert!(meta.caption_failed);
        assert_eq!(meta.caption, "");
        assert_eq!(meta.page, None);
    }

    #[tokio::test]
    async fn api_missing_response_field_is_an_internal_error() {
        let (url, _) =
            stub_server(axum::http::StatusCode::OK, r#"{"other":1}"#, "authorization").await;
        let svc = api_service(DocServiceApiConfig {
            url,
            response_field: "markdown".into(),
            ..Default::default()
        });
        let e = svc.to_markdown(b"x", "a.pdf").await.unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
        assert!(e.message.contains("markdown"), "{}", e.message);
    }

    #[tokio::test]
    async fn api_non_2xx_carries_the_status() {
        for (status, code_num) in [
            (axum::http::StatusCode::UNAUTHORIZED, 401),
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, 500),
        ] {
            let (url, _) = stub_server(status, "denied", "authorization").await;
            let svc = api_service(DocServiceApiConfig { url, ..Default::default() });
            let e = svc.to_markdown(b"x", "a.pdf").await.unwrap_err();
            assert_eq!(e.code, code::INTERNAL_ERROR);
            assert!(e.message.contains(&code_num.to_string()), "{}", e.message);
            assert!(e.message.contains("denied"), "{}", e.message);
        }
    }

    // ── opt in: the real converter ───────────────────────────────────────────

    /// Requires `doc-convert` on PATH and takes tens of seconds, so it is opt in:
    /// `cargo test -p mcp-fs --lib docs::service -- --ignored`. It skips rather
    /// than fails when the binary is absent, because "not installed" is not a
    /// regression.
    #[tokio::test]
    #[ignore = "needs a real doc-convert on PATH"]
    async fn real_doc_convert_produces_markdown_and_leaves_nothing_behind() {
        let Ok(binary) = which::which("doc-convert") else {
            eprintln!("skipped: doc-convert is not on PATH");
            return;
        };
        // A minimal one page PDF, built here so the repo carries no binary fixture.
        let pdf = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n\
4 0 obj<</Length 44>>stream\nBT /F1 24 Tf 20 100 Td (hello mcpfs) Tj ET\nendstream endobj\n\
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n\
trailer<</Root 1 0 R>>\n%%EOF\n";

        let svc = cli_service(
            vec![
                binary.display().to_string(),
                "--stdout".into(),
                "--quiet".into(),
                crate::config::DOC_PLACEHOLDER.into(),
            ],
            900,
        );
        let before = std::env::current_dir().unwrap();
        let md = svc.to_markdown(pdf, "sample.pdf").await.unwrap();
        assert!(!md.trim().is_empty(), "the converter produced nothing");

        // The `_docling` leftovers must have gone with the sandbox, never next to us.
        let leftovers: Vec<_> = std::fs::read_dir(&before)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with("_docling"))
            .collect();
        assert!(leftovers.is_empty(), "the converter escaped its sandbox: {leftovers:?}");
    }
}
