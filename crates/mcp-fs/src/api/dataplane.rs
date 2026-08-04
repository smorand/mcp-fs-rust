//! The `/api/fs` REST data plane. 1:1 port of the C# `Api/DataPlane.cs`.
//!
//! Two families of endpoints share one guard:
//!
//! * the bytes plane (`roots`, `list`, `upload`, `download`, `download-zip`,
//!   plus the small `mkdir` / `delete` / `move` helpers), which talks to the
//!   [`VolumeClient`] directly and returns the shapes the C# returns;
//! * tool parity (`read`, `write`, `grep`, ...), which calls the very same
//!   [`crate::core::fs_ops`] functions the MCP tools call, so the two surfaces
//!   can never drift apart.
//!
//! Guard order, identical to the C#: verify the bearer (401 on failure), then
//! `require_member` on the project (404 when the project is unknown, 403 when the
//! caller is not a member), then run the operation and map any [`ToolError`] to
//! its HTTP status via [`ToolError::http_status`].
//!
//! Query and body parameter names are the MCP tool parameter names, spelled
//! snake_case. That is a hard requirement: the OpenAPI document inherits its
//! descriptions by matching those names against the tool schemas (see
//! [`super::openapi`]), and a REST client that knows the tools knows this API.

use crate::core::fs_ops;
use crate::docs;
use crate::errors::{Result, ToolError};
use crate::mcp::Args;
use crate::safety::SafetyManager;
use crate::state::AppState;
use crate::storage::VolumeClient;
use crate::util::PosixPath;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::io::Write as _;
use std::sync::Arc;

/// Every route this module serves, as (HTTP method, last path segment).
///
/// The OpenAPI table in [`super::openapi`] is keyed by the same pairs, and a test
/// there asserts the two lists match, so a new endpoint cannot ship undocumented.
pub const REST_ROUTES: &[(&str, &str)] = &[
    // bytes plane
    ("GET", "roots"),
    ("GET", "list"),
    ("POST", "mkdir"),
    ("POST", "delete"),
    ("POST", "move"),
    ("POST", "upload"),
    ("GET", "download"),
    ("GET", "download-zip"),
    // tool parity, read side
    ("GET", "read"),
    ("GET", "read-bytes"),
    ("GET", "read-lines"),
    ("GET", "read-section"),
    ("GET", "head"),
    ("GET", "tail"),
    ("GET", "stat"),
    ("GET", "exists"),
    ("GET", "hash"),
    ("GET", "count-lines"),
    ("GET", "glob"),
    ("GET", "grep"),
    ("GET", "tree"),
    ("GET", "find-definition"),
    ("GET", "find-references"),
    ("GET", "audit-log"),
    // tool parity, write and compute side
    ("POST", "read-many"),
    ("POST", "write"),
    ("POST", "append"),
    ("POST", "create-empty"),
    ("POST", "copy"),
    ("POST", "edit"),
    ("POST", "multi-edit"),
    ("POST", "search-replace"),
    ("POST", "insert-at-line"),
    ("POST", "apply-patch"),
    ("POST", "extract-text"),
    ("POST", "write-docx"),
];

/// Request body ceiling for the JSON endpoints. Axum defaults to 2 MiB, which is
/// far below what the reference server accepts, so it is raised to Kestrel's own
/// default (`MaxRequestBodySize`) to keep a large `write` working on both.
const JSON_BODY_LIMIT: usize = 30 * 1024 * 1024;

/// Ceiling for the multipart upload, matching the ASP.NET Core default
/// `MultipartBodyLengthLimit` the C# runs with.
const UPLOAD_BODY_LIMIT: usize = 128 * 1024 * 1024;

/// The data plane router, already carrying the shared state.
///
/// Mount it with `Router::merge`. Paths are absolute (`/api/fs/...`) rather than
/// nested so the OpenAPI document and the router cannot disagree about a prefix.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // bytes plane
        .route("/api/fs/roots", get(roots))
        .route("/api/fs/{mount_id}/list", get(list))
        .route("/api/fs/{mount_id}/mkdir", post(mkdir))
        .route("/api/fs/{mount_id}/delete", post(delete))
        .route("/api/fs/{mount_id}/move", post(move_path))
        .route(
            "/api/fs/{mount_id}/upload",
            post(upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route("/api/fs/{mount_id}/download", get(download))
        .route("/api/fs/{mount_id}/download-zip", get(download_zip))
        // tool parity, read side
        .route("/api/fs/{mount_id}/read", get(read))
        .route("/api/fs/{mount_id}/read-bytes", get(read_bytes))
        .route("/api/fs/{mount_id}/read-lines", get(read_lines))
        .route("/api/fs/{mount_id}/read-section", get(read_section))
        .route("/api/fs/{mount_id}/head", get(head))
        .route("/api/fs/{mount_id}/tail", get(tail))
        .route("/api/fs/{mount_id}/stat", get(stat))
        .route("/api/fs/{mount_id}/exists", get(exists))
        .route("/api/fs/{mount_id}/hash", get(hash))
        .route("/api/fs/{mount_id}/count-lines", get(count_lines))
        .route("/api/fs/{mount_id}/glob", get(glob))
        .route("/api/fs/{mount_id}/grep", get(grep))
        .route("/api/fs/{mount_id}/tree", get(tree))
        .route("/api/fs/{mount_id}/find-definition", get(find_definition))
        .route("/api/fs/{mount_id}/find-references", get(find_references))
        .route("/api/fs/{mount_id}/audit-log", get(audit_log))
        // tool parity, write and compute side
        .route("/api/fs/{mount_id}/read-many", post(read_many))
        .route("/api/fs/{mount_id}/write", post(write))
        .route("/api/fs/{mount_id}/append", post(append))
        .route("/api/fs/{mount_id}/create-empty", post(create_empty))
        .route("/api/fs/{mount_id}/copy", post(copy))
        .route("/api/fs/{mount_id}/edit", post(edit))
        .route("/api/fs/{mount_id}/multi-edit", post(multi_edit))
        .route("/api/fs/{mount_id}/search-replace", post(search_replace))
        .route("/api/fs/{mount_id}/insert-at-line", post(insert_at_line))
        .route("/api/fs/{mount_id}/apply-patch", post(apply_patch))
        .route("/api/fs/{mount_id}/extract-text", post(extract_text))
        .route("/api/fs/{mount_id}/write-docx", post(write_docx))
        // Applied outside the routes so the upload keeps its own larger ceiling.
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        .with_state(state)
}

// ────────────────────────────────────────────────────────────────── guards ────

/// Everything a guarded handler needs: the caller, the volume, the shared state.
struct Req {
    state: Arc<AppState>,
    person: String,
    mount: String,
    client: Arc<VolumeClient>,
}

impl Req {
    /// Normalize an in-volume path (the C# `ToolContext.Norm`).
    fn norm(&self, path: &str) -> Result<String> {
        self.state.safety.normalize_path(path)
    }

    fn safety(&self) -> &SafetyManager {
        &self.state.safety
    }
}

/// Bearer verification only, for the one route that is not scoped to a project.
fn person_of(state: &AppState, headers: &HeaderMap) -> Result<String> {
    state.identity.resolve(|name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    })
}

/// Bearer, then the project membership gate, then the operation.
async fn guarded<F, Fut>(state: Arc<AppState>, headers: HeaderMap, mount: String, op: F) -> Response
where
    F: FnOnce(Req) -> Fut,
    Fut: Future<Output = Result<Response>>,
{
    let person = match person_of(&state, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    let client = match state.authorize(&mount, &person).await {
        Err(e) => return error_response(&e),
        Ok(()) => match state.stores.client(&mount).await {
            Ok(c) => c,
            Err(e) => return error_response(&e),
        },
    };
    let req = Req { state, person, mount, client };
    match op(req).await {
        Ok(r) => r,
        Err(e) => error_response(&e),
    }
}

/// A guarded handler that answers with the tool payload as JSON.
async fn guarded_json<F, Fut>(state: Arc<AppState>, headers: HeaderMap, mount: String, op: F) -> Response
where
    F: FnOnce(Req) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    guarded(state, headers, mount, |r| async move { Ok(Json(op(r).await?).into_response()) }).await
}

/// The C# 401 body. `detail` carries the full `CODE: message` rendering, verified
/// against the reference server: `{"error":"ERR_X","detail":"ERR_X: message"}`.
fn unauthorized(err: &ToolError) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": err.code, "detail": err.to_string()})),
    )
        .into_response()
}

/// Map a tool error to its HTTP status, keeping the stable `ERR_*` code in the body.
fn error_response(err: &ToolError) -> Response {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    // `detail` repeats the code, matching the reference server exactly.
    (status, Json(json!({"error": err.code, "detail": err.to_string()}))).into_response()
}

/// The C# `Results.NotFound(new { detail })` shape, used by the bytes plane where
/// the reference implementation short circuits instead of raising a tool error.
fn not_found_detail(detail: String) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"detail": detail}))).into_response()
}

// ───────────────────────────────────────────────────── query and body args ────

/// Query string accessors with the HTTP semantics the C# model binder has: a
/// missing required value or an unparsable one is `ERR_INVALID_ARGUMENT` (400),
/// never a silent default.
struct Q(Vec<(String, String)>);

impl Q {
    fn opt(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, v)| k == key && !v.is_empty())
            .map(|(_, v)| v.as_str())
    }

    /// Every value given for `key`, in order (`?x=a&x=b`), like `Query[key]` in C#.
    fn all(&self, key: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(k, v)| k == key && !v.is_empty())
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn req_str(&self, key: &str) -> Result<&str> {
        self.opt(key)
            .ok_or_else(|| ToolError::invalid_argument(format!("missing required argument '{key}'")))
    }

    fn str_or(&self, key: &str, default: &'static str) -> &str {
        self.opt(key).unwrap_or(default)
    }

    fn int_or(&self, key: &str, default: i64) -> Result<i64> {
        match self.opt(key) {
            None => Ok(default),
            Some(v) => v
                .parse::<i64>()
                .map_err(|_| ToolError::invalid_argument(format!("argument '{key}' must be an integer"))),
        }
    }

    fn req_int(&self, key: &str) -> Result<i64> {
        self.req_str(key)?
            .parse::<i64>()
            .map_err(|_| ToolError::invalid_argument(format!("argument '{key}' must be an integer")))
    }

    fn bool_or(&self, key: &str, default: bool) -> Result<bool> {
        match self.opt(key) {
            None => Ok(default),
            Some(v) => match v.to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(ToolError::invalid_argument(format!(
                    "argument '{key}' must be a boolean"
                ))),
            },
        }
    }

    fn num_opt(&self, key: &str) -> Result<Option<f64>> {
        match self.opt(key) {
            None => Ok(None),
            Some(v) => v
                .parse::<f64>()
                .map(Some)
                .map_err(|_| ToolError::invalid_argument(format!("argument '{key}' must be a number"))),
        }
    }
}

/// Parse a JSON request body into the same tolerant accessors the tools use, so
/// a REST body and a `tools/call` argument object behave identically.
fn body_args(body: &Bytes) -> Result<Args> {
    if body.is_empty() {
        return Ok(Args::new(Value::Null));
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| ToolError::invalid_argument(format!("invalid JSON body: {e}")))?;
    Ok(Args::new(value))
}

// ───────────────────────────────────────────────────────────── bytes plane ────

/// Projects the caller can reach. No membership gate: the answer IS the ACL.
async fn roots(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let person = match person_of(&state, &headers) {
        Ok(p) => p,
        Err(e) => return unauthorized(&e),
    };
    match state.admin.list_projects_for(&person).await {
        Err(e) => error_response(&e),
        Ok(projects) => {
            let roots: Vec<Value> = projects
                .iter()
                .map(|p| json!({"mount_id": p.id, "owner": p.owner}))
                .collect();
            Json(json!({"person": person, "roots": roots})).into_response()
        }
    }
}

/// Directory listing for a browser: directories first, then names caselessly.
async fn list(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.str_or("path", "/"))?;
        let mut rows = r.client.list_dir(&norm).await?;
        rows.sort_by(|a, b| {
            (!a.is_dir(), a.name.to_lowercase()).cmp(&(!b.is_dir(), b.name.to_lowercase()))
        });
        let entries: Vec<Value> = rows
            .iter()
            .map(|e| json!({"name": e.name, "kind": e.kind, "size": e.size, "mtime": e.mtime}))
            .collect();
        Ok(json!({"path": norm, "entries": entries}))
    })
    .await
}

/// Create a directory. Same parameters and audit trail as the tool: this used to call
/// the volume client directly, so `parents` and `exist_ok` were ignored and the
/// mutation left no trace.
async fn mkdir(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::mkdir(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            a.bool_or("parents", true),
            a.bool_or("exist_ok", true),
        )
        .await
    })
    .await
}

/// Delete a path, with the same `recursive` and `trash` semantics as the tool.
async fn delete(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        // Route through the engine like every other operation. Calling the volume
        // client directly (as this did) skipped the trash, ignored
        // safety.allow_hard_delete, let a whole tree go without `recursive`, and
        // wrote no audit entry: the same delete through two doors behaved
        // differently, and the REST door was the destructive one.
        fs_ops::delete_path(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            a.bool_or("recursive", false),
            a.bool_or("trash", true),
        )
        .await
    })
    .await
}

/// Rename or relocate. Named `move_path` because `move` is a Rust keyword.
async fn move_path(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let src = r.norm(&a.str("source")?)?;
        let dst = r.norm(&a.str("destination")?)?;
        // Same reasoning as delete: the engine owns the no clobber rule and the audit
        // entry, so the REST and MCP doors cannot drift apart.
        fs_ops::move_path(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &src,
            &dst,
            a.bool_or("overwrite", false),
        )
        .await
    })
    .await
}

/// Multipart upload. Any part carrying a filename is a file; `directory` is the
/// destination root and the optional repeated `paths` field gives per file
/// relative paths, which is how a folder upload preserves its structure.
async fn upload(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let mut multipart = multipart;
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut directory = "/".to_string();
        let mut rel_paths: Vec<String> = Vec::new();

        // The whole form is read first: the C# `ReadFormAsync` sees every field
        // before pairing files with `paths`, so part order must not matter.
        loop {
            let field = multipart
                .next_field()
                .await
                .map_err(|e| ToolError::invalid_argument(format!("invalid multipart body: {e}")))?;
            let Some(field) = field else { break };
            let name = field.name().unwrap_or_default().to_string();
            let file_name = field.file_name().map(str::to_string);
            match file_name {
                Some(fname) => {
                    let data = field.bytes().await.map_err(|e| {
                        ToolError::invalid_argument(format!("cannot read uploaded file: {e}"))
                    })?;
                    files.push((fname, data.to_vec()));
                }
                None => {
                    let text = field.text().await.map_err(|e| {
                        ToolError::invalid_argument(format!("cannot read form field '{name}': {e}"))
                    })?;
                    match name.as_str() {
                        "directory" if !text.is_empty() => directory = text,
                        "paths" => rel_paths.push(text),
                        _ => {}
                    }
                }
            }
        }

        let base = r.norm(&directory)?;
        let mut written: Vec<String> = Vec::new();
        for (index, (fname, data)) in files.iter().enumerate() {
            let rel = match rel_paths.get(index) {
                Some(p) if !p.is_empty() => p.as_str(),
                _ if !fname.is_empty() => fname.as_str(),
                _ => "file",
            };
            let joined = format!("{}/{}", base.trim_end_matches('/'), rel);
            let dest = r.norm(&PosixPath::normpath(&joined))?;
            // Through the engine so an upload is charged against the write quota and
            // audited. Writing via the volume client (as this did) made the highest
            // volume write path the only one with no accounting at all.
            fs_ops::write_bytes(
                &r.client,
                r.safety(),
                &r.person,
                &r.mount,
                &dest,
                data,
                true,
                true,
            )
            .await?;
            written.push(dest);
        }
        Ok(json!({"written": written, "count": written.len()}))
    })
    .await
}

/// One file's raw bytes as an attachment, with a guessed content type.
async fn download(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        if !r.client.is_file(&norm).await? {
            return Ok(not_found_detail(format!("not a file: {norm}")));
        }
        let data = r.client.read_bytes(&norm).await?;
        let name = PosixPath::basename(&norm);
        let mime = docs::guess_mime(&norm).unwrap_or("application/octet-stream");
        Ok(attachment(data, mime, &name))
    })
    .await
}

/// A subtree as a zip archive, entry names relative to the requested root.
async fn download_zip(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded(state, headers, mount, |r| async move {
        let root = r.norm(q.str_or("path", "/"))?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (dirpath, _, filenames) in r.client.walk(&root).await? {
            for name in filenames {
                let full = format!("{}/{}", dirpath.trim_end_matches('/'), name);
                let mut arcname = full[root.len().min(full.len())..].trim_start_matches('/');
                if arcname.is_empty() {
                    arcname = &name;
                }
                let data = r.client.read_bytes(&full).await?;
                zip.start_file(arcname, options).map_err(zip_error)?;
                zip.write_all(&data).map_err(|e| ToolError::internal(e.to_string()))?;
            }
        }
        let bytes = zip.finish().map_err(zip_error)?.into_inner();
        let mut label = PosixPath::basename(root.trim_end_matches('/'));
        if label.is_empty() {
            label = r.mount.clone();
        }
        Ok(attachment(bytes, "application/zip", &format!("{label}.zip")))
    })
    .await
}

fn zip_error(e: zip::result::ZipError) -> ToolError {
    ToolError::internal(format!("zip: {e}"))
}

/// An attachment response. The filename is sent twice (a sanitized ASCII form
/// plus RFC 5987 `filename*`) so unicode names survive without breaking headers.
fn attachment(data: Vec<u8>, mime: &str, name: &str) -> Response {
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' { c } else { '_' })
        .collect();
    let mut encoded = String::new();
    for b in name.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*b as char);
            }
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"),
            ),
        ],
        data,
    )
        .into_response()
}

// ───────────────────────────────────────────────────── tool parity: reads ─────

async fn read(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::read_window(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            q.int_or("offset_lines", 0)?,
            q.int_or("limit_lines", 2000)?,
            q.bool_or("line_numbered", true)?,
        )
        .await
    })
    .await
}

async fn read_bytes(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::read_bytes_b64(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            q.int_or("offset_bytes", 0)?,
            q.int_or("length_bytes", 65536)?,
        )
        .await
    })
    .await
}

async fn read_lines(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::read_lines(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            q.req_int("start_line")?,
            q.req_int("end_line")?,
        )
        .await
    })
    .await
}

async fn read_section(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::read_section(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            q.req_int("anchor_line")?,
            q.int_or("max_lines", 200)?,
        )
        .await
    })
    .await
}

async fn head(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::head(&r.client, r.safety(), &r.person, &r.mount, &norm, q.int_or("lines", 20)?).await
    })
    .await
}

async fn tail(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::tail(&r.client, r.safety(), &r.person, &r.mount, &norm, q.int_or("lines", 20)?).await
    })
    .await
}

async fn stat(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::stat_info(&r.client, &norm).await
    })
    .await
}

async fn exists(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::exists_info(&r.client, &norm).await
    })
    .await
}

async fn hash(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::hash_file(&r.client, &norm, q.str_or("algo", "sha256")).await
    })
    .await
}

async fn count_lines(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.req_str("path")?)?;
        fs_ops::count_lines(&r.client, &norm).await
    })
    .await
}

async fn glob(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let root = r.norm(q.str_or("root", "/"))?;
        let pattern = q.req_str("pattern")?.to_string();
        fs_ops::glob_files(&r.client, &root, &pattern, &q.all("exclude_patterns")).await
    })
    .await
}

async fn grep(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let root = r.norm(q.str_or("root", "/"))?;
        fs_ops::grep_files(
            &r.client,
            &root,
            q.req_str("pattern")?,
            q.opt("include_glob"),
            q.opt("exclude_glob"),
            q.bool_or("regex", true)?,
            q.bool_or("case_sensitive", true)?,
            q.str_or("output_mode", "content"),
            q.int_or("context_lines", 0)?,
            q.int_or("max_matches", 100)?,
        )
        .await
    })
    .await
}

async fn tree(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let norm = r.norm(q.str_or("path", "/"))?;
        fs_ops::tree(
            &r.client,
            &norm,
            q.int_or("max_depth", 3)?,
            &q.all("exclude_patterns"),
            q.bool_or("with_sizes", false)?,
        )
        .await
    })
    .await
}

async fn find_definition(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let root = r.norm(q.str_or("root", "/"))?;
        fs_ops::find_definitions(&r.client, &root, q.req_str("name")?, q.opt("kind")).await
    })
    .await
}

async fn find_references(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let root = r.norm(q.str_or("root", "/"))?;
        fs_ops::find_references(&r.client, &root, q.req_str("name")?).await
    })
    .await
}

async fn audit_log(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    Query(q): Query<Vec<(String, String)>>,
) -> Response {
    let q = Q(q);
    guarded_json(state, headers, mount, |r| async move {
        let since = q.num_opt("since")?;
        let limit = q.int_or("limit", 20)?.max(0) as usize;
        let mut entries = r.safety().audit(&r.person, &r.mount);
        if let Some(floor) = since {
            entries.retain(|e| e.timestamp >= floor);
        }
        let skip = entries.len().saturating_sub(limit);
        Ok(json!({"entries": &entries[skip..]}))
    })
    .await
}

// ────────────────────────────────────────── tool parity: writes and compute ───

async fn read_many(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        fs_ops::read_many(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &a.req_str_array("paths")?,
            a.int_or("per_file_cap_lines", 500),
        )
        .await
    })
    .await
}

async fn write(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::write_text(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            &a.str("content")?,
            a.bool_or("overwrite", false),
            a.bool_or("create_parents", true),
        )
        .await
    })
    .await
}

async fn append(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::append_text(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            &a.str("content")?,
            a.bool_or("create", false),
        )
        .await
    })
    .await
}

async fn create_empty(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::create_empty(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            a.bool_or("exist_ok", false),
        )
        .await
    })
    .await
}

async fn copy(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let src = r.norm(&a.str("source")?)?;
        let dst = r.norm(&a.str("destination")?)?;
        fs_ops::copy_path(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &src,
            &dst,
            a.bool_or("overwrite", false),
            a.bool_or("recursive", false),
        )
        .await
    })
    .await
}

async fn edit(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::edit_unique(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            &a.str("old_string")?,
            &a.str("new_string")?,
            a.bool_or("replace_all", false),
            a.bool_or("dry_run", false),
        )
        .await
    })
    .await
}

async fn multi_edit(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        let edits = match a.raw("edits") {
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Err(ToolError::invalid_argument("argument 'edits' must be an array")),
            None => return Err(ToolError::invalid_argument("missing required argument 'edits'")),
        };
        fs_ops::multi_edit(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            &edits,
            a.bool_or("dry_run", false),
        )
        .await
    })
    .await
}

async fn search_replace(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::search_replace(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            &a.str("search_block")?,
            &a.str("replace_block")?,
            a.bool_or("fuzzy", false),
        )
        .await
    })
    .await
}

async fn insert_at_line(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::insert_at_line(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            a.int("line")?,
            &a.str("content")?,
        )
        .await
    })
    .await
}

/// Multi file V4A patch. Calls the engine directly, like every other route: this
/// used to dispatch through the tool registry because the V4A parser lived in the
/// tool module.
async fn apply_patch(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        fs_ops::apply_patch(&r.client, r.safety(), &r.person, &r.mount, &a.str("patch_text")?)
            .await
    })
    .await
}

async fn extract_text(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::extract_document(
            &r.client,
            r.safety(),
            &r.state.config.extract.ocr,
            &r.person,
            &r.mount,
            &norm,
            a.int_or("max_chars", 200_000).max(0) as usize,
            a.int_or("preview_chars", 4_000).max(0) as usize,
            a.bool_or("ocr", true),
            a.bool_or("refresh", false),
        )
        .await
    })
    .await
}

async fn write_docx(
    State(state): State<Arc<AppState>>,
    Path(mount): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    guarded_json(state, headers, mount, |r| async move {
        let a = body_args(&body)?;
        let norm = r.norm(&a.str("path")?)?;
        fs_ops::write_docx(
            &r.client,
            r.safety(),
            &r.person,
            &r.mount,
            &norm,
            &a.str("markdown")?,
            a.opt_str("title").as_deref(),
            a.bool_or("overwrite", false),
        )
        .await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::errors::code;
    use crate::keys;
    use crate::mcp::registry::ToolRegistry;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    const OWNER: &str = "owner@test.com";
    const STRANGER: &str = "stranger@test.com";
    const MOUNT: &str = "proj";

    struct Harness {
        _dir: tempfile::TempDir,
        state: Arc<AppState>,
        owner_token: String,
        stranger_token: String,
    }

    impl Harness {
        /// A real `AppState` over throwaway directories, one project owned by
        /// `OWNER`, and signed tokens for a member and a non member.
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (key_path, pub_path) = keys::write_keypair(root.join("keys")).unwrap();
            let mint = |person: &str| {
                keys::mint_token_from_file(
                    &key_path,
                    person,
                    keys::DEFAULT_ISSUER,
                    keys::DEFAULT_CLAIM,
                    3600,
                )
                .unwrap()
            };

            let mut config = ServerConfig::default();
            config.auth.jwt.public_key_path = pub_path.display().to_string();
            config.infra.meta.dir = root.join("volumes").display().to_string();
            config.infra.blob.dir = root.join("blobs").display().to_string();
            config.infra.admin.path = root.join("admin.db").display().to_string();
            let config = Arc::new(config);

            let admin = crate::storage::build_admin_store(&config).unwrap();
            admin.connect().await.unwrap();
            admin.create_project(MOUNT, OWNER).await.unwrap();

            let state = Arc::new(AppState {
                config: config.clone(),
                admin,
                stores: Arc::new(crate::storage::StoreManager::new(config.clone())),
                safety: Arc::new(SafetyManager::new(config.safety.clone())),
                identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
                registry: Arc::new(ToolRegistry::new()),
            });

            Self {
                _dir: dir,
                state,
                owner_token: mint(OWNER),
                stranger_token: mint(STRANGER),
            }
        }

        fn app(&self) -> Router {
            router(self.state.clone())
        }

        async fn send(&self, req: Request<axum::body::Body>) -> (StatusCode, Vec<u8>) {
            let response = self.app().oneshot(req).await.unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (status, body.to_vec())
        }

        async fn get(&self, uri: &str) -> (StatusCode, Value) {
            let (s, b) = self
                .send(
                    Request::builder()
                        .uri(uri)
                        .header("Authorization", format!("Bearer {}", self.owner_token))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await;
            (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
        }

        async fn get_raw(&self, uri: &str) -> (StatusCode, Vec<u8>) {
            self.send(
                Request::builder()
                    .uri(uri)
                    .header("Authorization", format!("Bearer {}", self.owner_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
        }

        async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
            let (s, b) = self
                .send(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("Authorization", format!("Bearer {}", self.owner_token))
                        .header("Content-Type", "application/json")
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap(),
                )
                .await;
            (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
        }

        /// Seed a file straight through the volume, bypassing the REST plane.
        async fn seed(&self, path: &str, content: &str) {
            self.state
                .stores
                .client(MOUNT)
                .await
                .unwrap()
                .write_text_atomic(path, content)
                .await
                .unwrap();
        }
    }

    fn u(sub: &str) -> String {
        format!("/api/fs/{MOUNT}/{sub}")
    }

    // ── auth and authorization ───────────────────────────────────────────────

    #[tokio::test]
    async fn without_a_token_every_route_is_401_json() {
        let h = Harness::new().await;
        for uri in ["/api/fs/roots", &u("read?path=/a.txt"), &u("stat?path=/a.txt")] {
            let (status, body) = h
                .send(Request::builder().uri(uri).body(axum::body::Body::empty()).unwrap())
                .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "uri {uri}");
            let v: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["error"], code::UNAUTHENTICATED);
            assert!(v["detail"].is_string());
        }
    }

    #[tokio::test]
    async fn a_post_without_a_token_is_401() {
        let h = Harness::new().await;
        let (status, _) = h
            .send(
                Request::builder()
                    .method("POST")
                    .uri(u("write"))
                    .header("Content-Type", "application/json")
                    .body(axum::body::Body::from(r#"{"path":"/x.txt","content":"x"}"#))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_garbage_token_is_401() {
        let h = Harness::new().await;
        let (status, _) = h
            .send(
                Request::builder()
                    .uri(u("stat?path=/a.txt"))
                    .header("Authorization", "Bearer not.a.jwt")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_forwarded_header_is_accepted_too() {
        let h = Harness::new().await;
        let (status, _) = h
            .send(
                Request::builder()
                    .uri("/api/fs/roots")
                    .header("X-Forwarded-Authorization", format!("Bearer {}", h.owner_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_non_member_is_403() {
        let h = Harness::new().await;
        let (status, body) = h
            .send(
                Request::builder()
                    .uri(u("list"))
                    .header("Authorization", format!("Bearer {}", h.stranger_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], code::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_unknown_project_is_404() {
        let h = Harness::new().await;
        let (status, body) = h
            .send(
                Request::builder()
                    .uri("/api/fs/no-such-project/list")
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], code::PROJECT_NOT_FOUND);
    }

    #[tokio::test]
    async fn roots_lists_the_callers_projects() {
        let h = Harness::new().await;
        let (status, v) = h.get("/api/fs/roots").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["person"], OWNER);
        assert_eq!(v["roots"][0]["mount_id"], MOUNT);
        assert_eq!(v["roots"][0]["owner"], OWNER);
    }

    // ── error mapping ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_missing_file_is_404() {
        let h = Harness::new().await;
        let (status, v) = h.get(&u("read?path=/nope.txt")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], code::NOT_FOUND);
    }

    #[tokio::test]
    async fn writing_over_a_file_without_overwrite_is_409() {
        let h = Harness::new().await;
        h.seed("/clash.txt", "first").await;
        let (status, v) = h.post(&u("write"), json!({"path": "/clash.txt", "content": "second"})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["error"], code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn a_missing_query_argument_is_400() {
        let h = Harness::new().await;
        let (status, v) = h.get(&u("read")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
        assert!(v["detail"].as_str().unwrap().contains("path"));
    }

    #[tokio::test]
    async fn an_unparsable_number_is_400() {
        let h = Harness::new().await;
        h.seed("/a.txt", "one\ntwo\n").await;
        let (status, v) = h.get(&u("head?path=/a.txt&lines=many")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn an_unparsable_boolean_is_400() {
        let h = Harness::new().await;
        h.seed("/a.txt", "one\n").await;
        let (status, v) = h.get(&u("read?path=/a.txt&line_numbered=maybe")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn a_malformed_json_body_is_400() {
        let h = Harness::new().await;
        let (status, body) = h
            .send(
                Request::builder()
                    .method("POST")
                    .uri(u("write"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .header("Content-Type", "application/json")
                    .body(axum::body::Body::from("{not json"))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn a_missing_body_field_is_400() {
        let h = Harness::new().await;
        let (status, v) = h.post(&u("write"), json!({"path": "/x.txt"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn a_path_with_a_nul_byte_is_400() {
        let h = Harness::new().await;
        let (status, v) = h.get(&u("stat?path=/a%00b")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::PATH_OUT_OF_BOUNDS);
    }

    // ── round trips ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_then_read_round_trip() {
        let h = Harness::new().await;
        let (status, v) = h.post(&u("write"), json!({"path": "/rest.txt", "content": "via rest"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["path"], "/rest.txt");
        assert_eq!(v["bytes_written"], 8);
        assert_eq!(v["overwritten"], false);
        assert_eq!(v["diff"], "");

        let (status, v) = h.get(&u("read?path=/rest.txt")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["content"], "1\tvia rest");
        assert_eq!(v["total_lines"], 1);
        assert_eq!(v["truncated"], false);
        assert!(v["next_offset"].is_null());
    }

    #[tokio::test]
    async fn read_plain_drops_the_line_numbers() {
        let h = Harness::new().await;
        h.seed("/a.txt", "one\ntwo").await;
        let (_, v) = h.get(&u("read?path=/a.txt&line_numbered=false")).await;
        assert_eq!(v["content"], "one\ntwo");
    }

    #[tokio::test]
    async fn read_bytes_uses_the_tool_parameter_names() {
        let h = Harness::new().await;
        h.seed("/a.txt", "first").await;
        let (status, v) = h.get(&u("read-bytes?path=/a.txt&offset_bytes=0&length_bytes=4")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["base64"], "Zmlycw==");
        assert_eq!(v["mime_type"], "text/plain");
        assert_eq!(v["length"], 4);
    }

    #[tokio::test]
    async fn read_lines_and_read_section_and_head_and_tail() {
        let h = Harness::new().await;
        h.seed("/a.txt", "first\nhello\nrust\ntail").await;
        let (_, v) = h.get(&u("read-lines?path=/a.txt&start_line=2&end_line=3")).await;
        assert_eq!(v["content"], "2\thello\n3\trust");
        assert_eq!(v["total_lines"], 4);

        let (status, v) = h.get(&u("read-section?path=/a.txt&anchor_line=1")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(v["content"].as_str().unwrap().contains("first"));

        let (_, v) = h.get(&u("head?path=/a.txt&lines=1")).await;
        assert_eq!(v["content"], "1\tfirst");
        let (_, v) = h.get(&u("tail?path=/a.txt&lines=1")).await;
        assert_eq!(v["content"], "4\ttail");
    }

    #[tokio::test]
    async fn read_many_isolates_per_file_errors() {
        let h = Harness::new().await;
        h.seed("/a.txt", "one").await;
        let (status, v) = h
            .post(&u("read-many"), json!({"paths": ["/a.txt", "/missing.txt"]}))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["files"][0]["path"], "/a.txt");
        assert!(v["files"][1]["error"].is_string());
    }

    #[tokio::test]
    async fn stat_exists_hash_and_count_lines() {
        let h = Harness::new().await;
        h.seed("/a.txt", "one\ntwo\n").await;

        let (_, v) = h.get(&u("stat?path=/a.txt")).await;
        assert_eq!(v["path"], "/a.txt");
        assert_eq!(v["size"], 8);
        assert_eq!(v["mode"], "0o644");
        assert_eq!(v["kind"], "file");
        assert_eq!(v["uid"], 1000);

        let (_, v) = h.get(&u("exists?path=/a.txt")).await;
        assert_eq!(v["exists"], true);
        assert_eq!(v["kind"], "file");

        let (_, v) = h.get(&u("exists?path=/nope")).await;
        assert_eq!(v["exists"], false);
        assert!(v["kind"].is_null());

        let (_, v) = h.get(&u("hash?path=/a.txt")).await;
        assert_eq!(v["algo"], "sha256");
        assert_eq!(v["size"], 8);
        assert_eq!(v["hash"].as_str().unwrap().len(), 64);

        let (_, v) = h.get(&u("hash?path=/a.txt&algo=md5")).await;
        assert_eq!(v["algo"], "md5");
        assert_eq!(v["hash"].as_str().unwrap().len(), 32);

        let (_, v) = h.get(&u("count-lines?path=/a.txt")).await;
        assert_eq!(v["total_lines"], 2);
    }

    #[tokio::test]
    async fn an_unknown_hash_algorithm_is_400() {
        let h = Harness::new().await;
        h.seed("/a.txt", "x").await;
        let (status, v) = h.get(&u("hash?path=/a.txt&algo=crc32")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn list_puts_directories_first_then_names_caselessly() {
        let h = Harness::new().await;
        h.seed("/b.txt", "b").await;
        h.seed("/A.txt", "a").await;
        h.seed("/dir/inner.txt", "i").await;
        let (status, v) = h.get(&u("list?path=/")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["path"], "/");
        let names: Vec<&str> =
            v["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["dir", "A.txt", "b.txt"]);
        assert_eq!(v["entries"][0]["kind"], "dir");
        assert_eq!(v["entries"][2]["size"], 1);
        assert!(v["entries"][2]["mtime"].is_number());
    }

    #[tokio::test]
    async fn tree_glob_and_grep() {
        let h = Harness::new().await;
        h.seed("/src/app.py", "def hello(name):\n    total = 1\n    return total\n").await;
        h.seed("/src/lib.rs", "pub fn hello() {}\n").await;

        let (status, v) = h.get(&u("tree?path=/&max_depth=3")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["path"], "/");
        assert_eq!(v["truncated"], false);
        assert_eq!(v["tree"][0]["name"], "src");
        assert_eq!(v["tree"][0]["kind"], "dir");

        let (status, v) = h.get(&u("glob?pattern=**/*.py")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["matches"], json!(["/src/app.py"]));
        assert_eq!(v["truncated"], false);

        let (status, v) = h.get(&u("grep?pattern=total")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["matches"][0]["path"], "/src/app.py");
        assert_eq!(v["matches"][0]["line"], 2);

        let (_, v) = h.get(&u("grep?pattern=total&output_mode=count")).await;
        assert_eq!(v["count"], 2);
        assert_eq!(v["files"], 1);
    }

    #[tokio::test]
    async fn repeated_exclude_patterns_are_all_honoured() {
        let h = Harness::new().await;
        h.seed("/keep.py", "x").await;
        h.seed("/skip/one.py", "x").await;
        h.seed("/other/two.py", "x").await;
        let (_, v) = h
            .get(&u("glob?pattern=**/*.py&exclude_patterns=/skip/*&exclude_patterns=/other/*"))
            .await;
        assert_eq!(v["matches"], json!(["/keep.py"]));
    }

    #[tokio::test]
    async fn find_definition_and_find_references() {
        let h = Harness::new().await;
        h.seed("/src/app.py", "def hello(name):\n    total = 1\n    return total\n").await;
        let (status, v) = h.get(&u("find-definition?name=hello")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["definitions"][0]["path"], "/src/app.py");
        assert_eq!(v["definitions"][0]["name"], "hello");
        assert_eq!(v["definitions"][0]["line"], 1);

        let (status, v) = h.get(&u("find-references?name=total")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!v["references"].as_array().unwrap().is_empty());
        assert_eq!(v["references"][0]["path"], "/src/app.py");
    }

    #[tokio::test]
    async fn the_audit_log_reports_rest_mutations() {
        let h = Harness::new().await;
        h.post(&u("write"), json!({"path": "/a.txt", "content": "hello"})).await;
        h.post(&u("append"), json!({"path": "/a.txt", "content": "!"})).await;
        let (status, v) = h.get(&u("audit-log")).await;
        assert_eq!(status, StatusCode::OK);
        let ops: Vec<&str> =
            v["entries"].as_array().unwrap().iter().map(|e| e["op"].as_str().unwrap()).collect();
        assert_eq!(ops, vec!["write", "append"]);

        let (_, v) = h.get(&u("audit-log?limit=1")).await;
        assert_eq!(v["entries"].as_array().unwrap().len(), 1);
        assert_eq!(v["entries"][0]["op"], "append");

        let (_, v) = h.get(&u("audit-log?since=99999999999")).await;
        assert!(v["entries"].as_array().unwrap().is_empty());
    }

    // ── mutations ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn append_create_empty_copy_move_and_delete() {
        let h = Harness::new().await;
        let (status, v) = h.post(&u("append"), json!({"path": "/log.txt", "content": "a", "create": true})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["bytes_appended"], 1);

        let (status, v) = h.post(&u("create-empty"), json!({"path": "/e.txt"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["created"], true);

        let (status, v) = h.post(&u("copy"), json!({"source": "/log.txt", "destination": "/log2.txt"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["destination"], "/log2.txt");

        let (status, v) = h.post(&u("move"), json!({"source": "/log2.txt", "destination": "/moved.txt"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["source"], "/log2.txt");
        assert_eq!(v["destination"], "/moved.txt");

        // The engine payload: soft delete by default, so the file moves to the trash
        // instead of vanishing. The REST door used to bypass this entirely.
        let (status, v) = h.post(&u("delete"), json!({"path": "/moved.txt"})).await;
        assert_eq!(status, StatusCode::OK, "body: {v}");
        assert_eq!(v["path"], "/moved.txt");
        assert_eq!(v["trashed"], true);
        let trashed = v["trash_path"].as_str().expect("a trash path").to_string();

        let (_, v) = h.get(&u("exists?path=/moved.txt")).await;
        assert_eq!(v["exists"], false, "gone from its original path");
        let (_, v) = h.get(&format!("{}?path={}", u("exists"), trashed)).await;
        assert_eq!(v["exists"], true, "recoverable from the trash");
    }

    /// Delete and move go through the engine, so a missing path is the standard
    /// `ERR_NOT_FOUND` body rather than a bespoke short circuit.
    #[tokio::test]
    async fn deleting_or_moving_a_missing_path_is_404_with_the_error_code() {
        let h = Harness::new().await;
        let (status, v) = h.post(&u("delete"), json!({"path": "/nope.txt"})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], code::NOT_FOUND);

        let (status, v) = h
            .post(&u("move"), json!({"source": "/nope.txt", "destination": "/x.txt"}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], code::NOT_FOUND);
    }

    /// A directory needs `recursive`, exactly like the tool. The REST door used to
    /// delete a whole tree without asking, and without an audit entry.
    #[tokio::test]
    async fn deleting_a_directory_requires_recursive() {
        let h = Harness::new().await;
        h.seed("/d/inner/f.txt", "x").await;
        let (status, v) = h.post(&u("delete"), json!({"path": "/d"})).await;
        assert!(
            (400..500).contains(&status.as_u16()),
            "a non empty directory must be refused without recursive, got {status} {v}"
        );
        let (_, v) = h.get(&u("exists?path=/d/inner/f.txt")).await;
        assert_eq!(v["exists"], true, "nothing was deleted");
    }

    /// Move honours no clobber, like the tool.
    #[tokio::test]
    async fn moving_onto_an_existing_path_needs_overwrite() {
        let h = Harness::new().await;
        h.seed("/from.txt", "a").await;
        h.seed("/onto.txt", "b").await;
        let (status, _) = h
            .post(&u("move"), json!({"source": "/from.txt", "destination": "/onto.txt"}))
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let (status, _) = h
            .post(
                &u("move"),
                json!({"source": "/from.txt", "destination": "/onto.txt", "overwrite": true}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn mkdir_is_idempotent() {
        let h = Harness::new().await;
        for _ in 0..2 {
            let (status, v) = h.post(&u("mkdir"), json!({"path": "/rest-dir"})).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(v["path"], "/rest-dir");
            assert_eq!(v["created"], true);
        }
        let (_, v) = h.get(&u("exists?path=/rest-dir")).await;
        assert_eq!(v["kind"], "dir");
    }

    #[tokio::test]
    async fn delete_removes_a_directory_recursively() {
        let h = Harness::new().await;
        h.seed("/d/inner/f.txt", "x").await;
        let (status, v) =
            h.post(&u("delete"), json!({"path": "/d", "recursive": true})).await;
        assert_eq!(status, StatusCode::OK, "body: {v}");
        assert_eq!(v["path"], "/d");
        let (_, v) = h.get(&u("exists?path=/d/inner/f.txt")).await;
        assert_eq!(v["exists"], false);
    }

    #[tokio::test]
    async fn edit_multi_edit_search_replace_and_insert() {
        let h = Harness::new().await;
        h.post(&u("write"), json!({"path": "/a.txt", "content": "one\ntwo\nthree\n"})).await;

        let (status, v) = h
            .post(&u("edit"), json!({"path": "/a.txt", "old_string": "two", "new_string": "2"}))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["applied"], true);
        assert!(v["diff"].as_str().unwrap().contains("+2"));

        let (status, v) = h
            .post(
                &u("multi-edit"),
                json!({"path": "/a.txt", "edits": [
                    {"old_string": "one", "new_string": "1"},
                    {"old_string": "three", "new_string": "3"}
                ]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["edits"], 2);
        assert_eq!(v["applied"], true);

        let (status, v) = h
            .post(
                &u("search-replace"),
                json!({"path": "/a.txt", "search_block": "1\n2\n", "replace_block": "one\ntwo\n"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["applied"], true);

        let (status, v) = h
            .post(&u("insert-at-line"), json!({"path": "/a.txt", "line": 1, "content": "zero"}))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["line"], 1);

        let (_, v) = h.get(&u("read?path=/a.txt")).await;
        assert!(v["content"].as_str().unwrap().starts_with("1\tzero"));
    }

    #[tokio::test]
    async fn multi_edit_without_edits_is_400() {
        let h = Harness::new().await;
        h.post(&u("write"), json!({"path": "/a.txt", "content": "x"})).await;
        let (status, v) = h.post(&u("multi-edit"), json!({"path": "/a.txt"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn editing_a_file_never_read_is_blocked_by_the_read_guard() {
        let h = Harness::new().await;
        h.seed("/guard.txt", "x").await; // seeded outside the plane, so never read
        let (status, v) = h
            .post(&u("edit"), json!({"path": "/guard.txt", "old_string": "x", "new_string": "y"}))
            .await;
        // A precondition on session state the caller can satisfy, so a 4xx: the
        // reference reported 500 and looked like a server fault.
        assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(v["error"], code::EDIT_WITHOUT_PRIOR_READ);
    }

    #[tokio::test]
    async fn write_docx_renders_a_document() {
        let h = Harness::new().await;
        let (status, v) = h
            .post(&u("write-docx"), json!({"path": "/report.docx", "markdown": "# Title\n\nBody."}))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["path"], "/report.docx");
        assert_eq!(v["overwritten"], false);
        assert!(v["bytes_written"].as_i64().unwrap() > 0);

        let (status, body) = h.get_raw(&u("download?path=/report.docx")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..2], b"PK", "a docx is a zip");
    }

    #[tokio::test]
    async fn write_docx_rejects_another_extension() {
        let h = Harness::new().await;
        let (status, v) = h
            .post(&u("write-docx"), json!({"path": "/report.txt", "markdown": "x"}))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn extract_text_returns_a_preview() {
        let h = Harness::new().await;
        h.seed("/notes.txt", "hello from a text file").await;
        let (status, v) = h.post(&u("extract-text"), json!({"path": "/notes.txt"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["path"], "/notes.txt");
        assert_eq!(v["format"], "text");
        assert!(v["preview"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn extract_text_on_a_missing_file_is_404() {
        let h = Harness::new().await;
        let (status, v) = h.post(&u("extract-text"), json!({"path": "/nope.pdf"})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], code::NOT_FOUND);
    }

    /// The route calls `fs_ops::apply_patch` directly. It used to dispatch through the
    /// tool registry because the V4A parser lived in the tool module.
    #[tokio::test]
    async fn apply_patch_applies_a_real_patch() {
        let h = Harness::new().await;
        // Read it through the plane first so the guard is satisfied.
        h.seed("/p.txt", "one\ntwo\n").await;
        let (status, _) = h.get(&format!("{}?path=/p.txt", u("read"))).await;
        assert_eq!(status, StatusCode::OK);

        let patch = "*** Begin Patch\n\
                     *** Update File: /p.txt\n\
                     @@\n\
                     -two\n\
                     +TWO\n\
                     *** End Patch\n";
        let (status, v) = h.post(&u("apply-patch"), json!({"patch_text": patch})).await;
        assert_eq!(status, StatusCode::OK, "body: {v}");
        assert!(v["files"].is_array(), "one entry per touched path: {v}");

        let (_, after) = h.get(&format!("{}?path=/p.txt&line_numbered=false", u("read"))).await;
        assert_eq!(after["content"], "one\nTWO");
    }

    /// A malformed patch is the caller's fault, so a 4xx with a stable code.
    #[tokio::test]
    async fn apply_patch_rejects_a_malformed_envelope() {
        let h = Harness::new().await;
        let (status, v) = h.post(&u("apply-patch"), json!({"patch_text": "not a patch"})).await;
        assert!(
            (400..500).contains(&status.as_u16()),
            "expected a 4xx, got {status} with {v}"
        );
    }

    // ── bytes plane ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn upload_then_download_round_trip() {
        let h = Harness::new().await;
        let boundary = "X-BOUNDARY";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"directory\"\r\n\r\n/docs\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"note.txt\"\r\n\
             Content-Type: text/plain\r\n\r\nuploaded bytes\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let (status, raw) = h
            .send(
                Request::builder()
                    .method("POST")
                    .uri(u("upload"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["written"], json!(["/docs/note.txt"]));

        let (status, bytes) = h.get_raw(&u("download?path=/docs/note.txt")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, b"uploaded bytes");
    }

    /// An upload must be charged against the write quota and audited. It used to write
    /// through the volume client, so the highest volume write path had no accounting:
    /// a caller could push unlimited bytes and leave no trace.
    #[tokio::test]
    async fn upload_is_charged_against_the_quota_and_audited() {
        let h = Harness::new().await;
        let boundary = "Q-BOUNDARY";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"directory\"\r\n\r\n/up\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"m.txt\"\r\n\
             Content-Type: text/plain\r\n\r\n0123456789\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let (status, _) = h
            .send(
                Request::builder()
                    .method("POST")
                    .uri(u("upload"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(
            h.state.safety.bytes_written(OWNER, MOUNT),
            10,
            "the ten uploaded bytes must be charged"
        );
        let log = h.state.safety.audit(OWNER, MOUNT);
        let entry = log.last().expect("an audit entry for the upload");
        assert_eq!(entry.op, "write");
        assert_eq!(entry.path, "/up/m.txt");
        assert_eq!(entry.detail, "10 bytes");
    }

    /// mkdir goes through the engine, so it is audited like the tool.
    #[tokio::test]
    async fn mkdir_is_audited() {
        let h = Harness::new().await;
        let (status, _) = h.post(&u("mkdir"), json!({"path": "/audited-dir"})).await;
        assert_eq!(status, StatusCode::OK);
        let log = h.state.safety.audit(OWNER, MOUNT);
        assert!(
            log.iter().any(|e| e.op == "mkdir" && e.path == "/audited-dir"),
            "no mkdir audit entry: {log:?}"
        );
    }

    #[tokio::test]
    async fn upload_honours_per_file_relative_paths() {
        let h = Harness::new().await;
        let boundary = "B2";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"paths\"\r\n\r\nsub/deep.txt\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"ignored.txt\"\r\n\r\ndeep\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let (status, raw) = h
            .send(
                Request::builder()
                    .method("POST")
                    .uri(u("upload"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["written"], json!(["/sub/deep.txt"]));
    }

    #[tokio::test]
    async fn download_sets_the_content_type_and_an_attachment_name() {
        let h = Harness::new().await;
        h.seed("/page.html", "<h1>hi</h1>").await;
        let response = h
            .app()
            .oneshot(
                Request::builder()
                    .uri(u("download?path=/page.html"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        let disposition = response.headers()[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(disposition.starts_with("attachment; filename=\"page.html\""), "got {disposition}");
    }

    #[tokio::test]
    async fn downloading_a_directory_is_404_with_a_detail_body() {
        let h = Harness::new().await;
        h.seed("/d/f.txt", "x").await;
        let (status, raw) = h.get_raw(&u("download?path=/d")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["detail"], "not a file: /d");
    }

    #[tokio::test]
    async fn download_zip_returns_a_zip_archive() {
        let h = Harness::new().await;
        h.seed("/src/a.txt", "aaa").await;
        h.seed("/src/inner/b.txt", "bbb").await;
        let response = h
            .app()
            .oneshot(
                Request::builder()
                    .uri(u("download-zip?path=/src"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
        assert!(
            response.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .contains("src.zip")
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..2], b"PK");

        // The archive holds both files, named relative to the requested root.
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "inner/b.txt"]);
    }

    #[tokio::test]
    async fn download_zip_of_the_root_is_named_after_the_mount() {
        let h = Harness::new().await;
        h.seed("/a.txt", "x").await;
        let response = h
            .app()
            .oneshot(
                Request::builder()
                    .uri(u("download-zip"))
                    .header("Authorization", format!("Bearer {}", h.owner_token))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let disposition = response.headers()[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(disposition.contains("proj.zip"), "got {disposition}");
    }

    /// Axum's 2 MiB default would reject a body the reference server accepts.
    #[tokio::test]
    async fn a_body_larger_than_the_axum_default_is_accepted() {
        let h = Harness::new().await;
        let content = "x".repeat(3 * 1024 * 1024);
        let (status, v) = h.post(&u("write"), json!({"path": "/big.txt", "content": content})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["bytes_written"], 3 * 1024 * 1024);
    }

    #[tokio::test]
    async fn the_route_inventory_covers_every_registered_path() {
        // Guards the OpenAPI table: a route added here without a doc entry fails
        // the matching test in `super::openapi`.
        assert_eq!(REST_ROUTES.len(), 36);
        assert!(REST_ROUTES.contains(&("GET", "download-zip")));
        assert!(REST_ROUTES.contains(&("POST", "write-docx")));
    }
}
