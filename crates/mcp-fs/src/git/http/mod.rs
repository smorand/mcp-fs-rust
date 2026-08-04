//! Git smart HTTP protocol, version 0 (the "dumb prefix" advertisement form).
//!
//! Port of the C# `Git/Http/GitHttpHandler.cs`, `UploadPackService.cs` and
//! `ReceivePackService.cs`, kept in one module because upload-pack and
//! receive-pack share the ref advertisement plumbing.
//!
//! Routes (mounted by the composition root, see [`router`]):
//!
//! ```text
//! GET  /git/{mount_id}/info/refs?service=git-upload-pack     ref advertisement
//! GET  /git/{mount_id}/info/refs?service=git-receive-pack    ref advertisement
//! POST /git/{mount_id}/git-upload-pack                       clone / fetch
//! POST /git/{mount_id}/git-receive-pack                      push
//! ```
//!
//! Deliberate parity choices, all matching the C# byte for byte:
//!
//! * Capability strings are copied verbatim, including `agent=mcp-fs/0.1.0`.
//! * The pack is always full (no real have/want negotiation): every wanted tip is
//!   inserted recursively and a NAK is sent.
//! * The receive-pack report DOES send the `unpack ok` line that git's report-status
//!   requires. The C# omits it, which makes a real `git push` report a failure even
//!   though the refs update correctly. This is a deliberate divergence: reproducing a
//!   protocol bug that breaks a documented feature is not useful parity. Everything
//!   else about the report (per ref `ok`/`ng` lines, framing, flush) is unchanged.
//! * `401` responses carry `WWW-Authenticate: Bearer realm="mcp-fs"`, like the C#.
//!   Note the git CLI only prompts for credentials on a `Basic` challenge, so
//!   anonymous CLI use needs `git.anonymous_read` or an explicit credential helper.
//!
//! Two things are stricter than the C#, on purpose:
//!
//! * Project membership is enforced. The C# `IsAuthorizedAsync` only checked that
//!   the repo existed, so any verified token could read or write any project.
//! * `git.max_pack_size_mb` is enforced on push bodies (`413`). The C# parsed the
//!   setting and never used it.

pub mod pktline;

use crate::errors::{Result, ToolError};
use crate::git::db::GitRefRow;
use crate::git::repo::{GitRepoEntry, GitRepoStore};
use crate::state::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use git2::Oid;
use pktline::{PacketType, PktReader};
use serde::Deserialize;
use std::sync::Arc;

/// Advertised upload-pack capabilities.
///
/// `multi_ack_detailed` is REQUIRED by git over smart HTTP: with `--stateless-rpc` the
/// client refuses to negotiate without it ("the option '--stateless-rpc' requires
/// 'multi_ack_detailed'"). The reference implementation advertises only `multi_ack`, so a
/// real `git clone` fails against it; verified against both servers. Advertising the
/// detailed variant is a deliberate divergence: parity that leaves the documented git
/// protocol unusable is not useful parity.
const UPLOAD_PACK_CAPABILITIES: &str =
    "multi_ack multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0";
const RECEIVE_PACK_CAPABILITIES: &str =
    "report-status delete-refs side-band-64k quiet atomic ofs-delta agent=mcp-fs/0.1.0";
const ZERO_ID: &str = "0000000000000000000000000000000000000000";
/// side-band-64k: 64k minus the 4 byte pkt prefix and the 1 byte channel id.
const SIDE_BAND_CHUNK: usize = 65519;

// ── router ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct GitHttpState {
    pub app: Arc<AppState>,
    pub git: Arc<GitRepoStore>,
}

/// The git smart HTTP router, ready to `merge` into the main app.
///
/// ```ignore
/// let git = mcp_fs::git::repo::GitRepoStore::shared(state.config.clone());
/// if state.config.git.enabled {
///     app = app.merge(mcp_fs::git::http::router(state.clone(), git));
/// }
/// ```
pub fn router(app: Arc<AppState>, git: Arc<GitRepoStore>) -> Router {
    // Push bodies are packfiles, far above axum's 2 MB default. The layer bound and
    // the explicit check below both come from git.max_pack_size_mb.
    let max_pack = max_pack_bytes(&app);
    let state = GitHttpState { app, git };
    Router::new()
        .route("/git/{mount_id}/info/refs", get(info_refs))
        .route("/git/{mount_id}/git-upload-pack", post(upload_pack))
        .route(
            "/git/{mount_id}/git-receive-pack",
            post(receive_pack).route_layer(DefaultBodyLimit::max(max_pack)),
        )
        .with_state(state)
}

/// [`router`] using the process wide store, for callers that have only the state.
pub fn router_with_shared_store(app: Arc<AppState>) -> Router {
    let git = GitRepoStore::shared(app.config.clone());
    router(app, git)
}

fn max_pack_bytes(app: &AppState) -> usize {
    (app.config.git.max_pack_size_mb as usize).saturating_mul(1024 * 1024)
}

// ── handlers ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ServiceQuery {
    pub service: Option<String>,
}

async fn info_refs(
    State(st): State<GitHttpState>,
    Path(mount_id): Path<String>,
    Query(q): Query<ServiceQuery>,
    headers: HeaderMap,
) -> Response {
    let service = q.service.unwrap_or_default();
    let is_upload = service == "git-upload-pack";
    let is_receive = service == "git-receive-pack";
    if !is_upload && !is_receive {
        return text(StatusCode::BAD_REQUEST, "service parameter required");
    }

    if let Err(resp) = gate(&st, &mount_id, &headers, is_upload).await {
        return resp;
    }

    let entry = match st.git.get_or_open_repo(&mount_id).await {
        Ok(e) => e,
        Err(e) => return error_response(&e),
    };

    let body = on_git_thread(move || async move {
        if is_upload {
            advertise_upload_pack(&entry).await
        } else {
            advertise_receive_pack(&entry).await
        }
    })
    .await;
    match body {
        Ok(bytes) => {
            let ct = if is_upload {
                "application/x-git-upload-pack-advertisement"
            } else {
                "application/x-git-receive-pack-advertisement"
            };
            git_response(ct, bytes)
        }
        Err(e) => error_response(&e),
    }
}

async fn upload_pack(
    State(st): State<GitHttpState>,
    Path(mount_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = gate(&st, &mount_id, &headers, true).await {
        return resp;
    }
    let entry = match st.git.get_or_open_repo(&mount_id).await {
        Ok(e) => e,
        Err(e) => return error_response(&e),
    };
    let body = body.to_vec();
    match on_git_thread(move || async move { handle_upload_pack(&entry, &body).await }).await {
        Ok(out) => git_response("application/x-git-upload-pack-result", out),
        Err(e) => error_response(&e),
    }
}

async fn receive_pack(
    State(st): State<GitHttpState>,
    Path(mount_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Push always needs an identity: anonymous_read is read only, by definition.
    if let Err(resp) = gate(&st, &mount_id, &headers, false).await {
        return resp;
    }
    let max = max_pack_bytes(&st.app);
    if body.len() > max {
        return text(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!(
                "pack exceeds git.max_pack_size_mb ({} MB)",
                st.app.config.git.max_pack_size_mb
            ),
        );
    }
    let entry = match st.git.get_or_open_repo(&mount_id).await {
        Ok(e) => e,
        Err(e) => return error_response(&e),
    };
    let body = body.to_vec();
    match on_git_thread(move || async move { handle_receive_pack(&entry, &body).await }).await {
        Ok(out) => git_response("application/x-git-receive-pack-result", out),
        Err(e) => error_response(&e),
    }
}

/// Run a libgit2 touching future on a blocking thread.
///
/// `git2::Repository` is `Send` but not `Sync`, so a future holding a reference to
/// one is not `Send` and axum cannot await it. Moving the work to the blocking
/// pool also mirrors the C#, where libgit2 runs on native threads and blocks on
/// the async blob store with `GetAwaiter().GetResult()`. As a bonus, pack building
/// (CPU bound, potentially seconds) stops occupying an async worker.
async fn on_git_thread<T, F, Fut>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>>,
{
    tokio::task::spawn_blocking(move || tokio::runtime::Handle::current().block_on(f()))
        .await
        .map_err(|e| ToolError::internal(format!("git task join: {e}")))?
}

// ── auth ────────────────────────────────────────────────────────────────────

/// Resolve the caller, enforce membership and require the repo to exist.
/// `Ok(Some(person))` for an authenticated caller, `Ok(None)` for an allowed
/// anonymous read, `Err(response)` when the request must be refused.
async fn gate(
    st: &GitHttpState,
    mount_id: &str,
    headers: &HeaderMap,
    is_read: bool,
) -> std::result::Result<Option<String>, Response> {
    let person = st.app.identity.resolve(|name| header_value(headers, name)).ok();

    match &person {
        None => {
            if !(is_read && st.app.config.git.anonymous_read) {
                let mut resp = text(StatusCode::UNAUTHORIZED, "authentication required");
                resp.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    header::HeaderValue::from_static("Bearer realm=\"mcp-fs\""),
                );
                return Err(resp);
            }
        }
        Some(p) => {
            // Not in the C#: a verified token alone used to grant access to every
            // project. Git traffic is project data, so it needs membership.
            if let Err(e) = st.app.admin.require_member(mount_id, p).await {
                return Err(error_response(&e));
            }
        }
    }

    if !st.git.is_initialized(mount_id).await {
        return Err(text(
            StatusCode::NOT_FOUND,
            &format!("repository '{mount_id}' not found"),
        ));
    }
    Ok(person)
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

// ── responses ───────────────────────────────────────────────────────────────

fn text(status: StatusCode, body: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(body.to_string()))
        .expect("static response is always valid")
}

fn error_response(e: &ToolError) -> Response {
    let status =
        StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    text(status, &e.to_string())
}

/// A git protocol response with the C# no-cache header trio.
fn git_response(content_type: &'static str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::EXPIRES, "Fri, 01 Jan 1980 00:00:00 GMT")
        .header(header::PRAGMA, "no-cache")
        .header(header::CACHE_CONTROL, "no-cache, max-age=0, must-revalidate")
        .body(Body::from(body))
        .expect("static response is always valid")
}

// ── ref advertisement ───────────────────────────────────────────────────────

/// Concrete `(ref name, sha)` pairs, C# `ResolveConcreteRefs` / `BuildConcreteRefs`.
/// `include_head` adds the resolved `HEAD` first, which upload-pack does and
/// receive-pack does not.
pub async fn concrete_refs(
    entry: &GitRepoEntry,
    include_head: bool,
) -> Result<Vec<(String, String)>> {
    let rows: Vec<GitRefRow> = entry.db.list_refs().await?;
    let mut out: Vec<(String, String)> = Vec::new();

    let direct: Vec<&GitRefRow> = rows.iter().filter(|r| !r.symbolic).collect();
    if include_head
        && let Some(head) = rows.iter().find(|r| r.symbolic && r.name == "HEAD")
        && let Some(target) = direct.iter().find(|r| r.name == head.target)
    {
        out.push(("HEAD".to_string(), target.target.clone()));
    }
    for r in &direct {
        out.push((r.name.clone(), r.target.clone()));
    }

    // Anything libgit2 knows about but the index does not (a ref created by a tool
    // through the repository handle directly).
    {
        let repo = entry.repo.lock().await;
        if let Ok(refs) = repo.references() {
            for r in refs.flatten() {
                let Some(name) = r.name() else { continue };
                if direct.iter().any(|d| d.name == name) {
                    continue;
                }
                if let Ok(resolved) = r.resolve()
                    && let Some(oid) = resolved.target()
                {
                    out.push((name.to_string(), oid.to_string()));
                }
            }
        }
    }

    // DistinctBy(name), keeping the first occurrence.
    let mut seen = std::collections::HashSet::new();
    out.retain(|(name, _)| seen.insert(name.clone()));
    Ok(out)
}

fn advertisement(service: &str, capabilities: &str, refs: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&pktline::encode(&format!("# service={service}\n")));
    out.extend_from_slice(&pktline::flush());
    if refs.is_empty() {
        // Empty repo: advertise capabilities against the zero id.
        let line = format!("{ZERO_ID} capabilities^{{}}\0{capabilities}\n");
        out.extend_from_slice(&pktline::encode_raw(line.as_bytes()));
    } else {
        for (i, (name, sha)) in refs.iter().enumerate() {
            let line = if i == 0 {
                format!("{sha} {name}\0{capabilities}\n")
            } else {
                format!("{sha} {name}\n")
            };
            out.extend_from_slice(&pktline::encode_raw(line.as_bytes()));
        }
    }
    out.extend_from_slice(&pktline::flush());
    out
}

pub async fn advertise_upload_pack(entry: &GitRepoEntry) -> Result<Vec<u8>> {
    let refs = concrete_refs(entry, true).await?;
    Ok(advertisement("git-upload-pack", UPLOAD_PACK_CAPABILITIES, &refs))
}

pub async fn advertise_receive_pack(entry: &GitRepoEntry) -> Result<Vec<u8>> {
    let refs = concrete_refs(entry, false).await?;
    Ok(advertisement("git-receive-pack", RECEIVE_PACK_CAPABILITIES, &refs))
}

// ── upload-pack ─────────────────────────────────────────────────────────────

/// Parsed `git-upload-pack` request.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UploadPackRequest {
    pub wants: Vec<String>,
    pub haves: Vec<String>,
}

/// Parse the want/have section, C# parity: stop at the first flush or `done`.
pub fn parse_upload_pack_request(body: &[u8]) -> UploadPackRequest {
    let mut req = UploadPackRequest::default();
    let mut reader = PktReader::new(body);
    loop {
        let (line, kind) = reader.read_line();
        if kind == PacketType::Flush {
            break;
        }
        let Some(line) = line else { break };
        if let Some(rest) = line.strip_prefix("want ") {
            // The first want carries the client capabilities after a space.
            let sha = rest.trim().split(' ').next().unwrap_or("").to_string();
            req.wants.push(sha);
        } else if let Some(rest) = line.strip_prefix("have ") {
            req.haves.push(rest.trim().to_string());
        } else if line == "done" {
            break;
        }
    }
    req
}

/// Serve a clone or fetch. Returns the full response body.
pub async fn handle_upload_pack(entry: &GitRepoEntry, body: &[u8]) -> Result<Vec<u8>> {
    let req = parse_upload_pack_request(body);
    let mut out = Vec::new();
    if req.wants.is_empty() {
        out.extend_from_slice(&pktline::flush());
        return Ok(out);
    }

    // No negotiation: acknowledge nothing and send everything reachable.
    out.extend_from_slice(&pktline::encode("NAK\n"));

    // libgit2 packs from its own ODB, so the blob store has to be materialized first.
    let repo = entry.repo.lock().await;
    entry.objects.export_to_repo(&repo).await?;

    let pack = build_pack(&repo, &req.wants)?;
    drop(repo);

    for chunk in pack.chunks(SIDE_BAND_CHUNK) {
        let mut framed = Vec::with_capacity(chunk.len() + 1);
        framed.push(1u8); // channel 1: packfile data
        framed.extend_from_slice(chunk);
        out.extend_from_slice(&pktline::encode_raw(&framed));
    }
    // C# writes a bare channel byte to close the side-band, then flushes.
    out.extend_from_slice(&pktline::encode("\u{1}"));
    out.extend_from_slice(&pktline::flush());
    Ok(out)
}

/// Build the packfile for a set of wanted tips.
///
/// The pack must contain the wanted objects AND everything reachable from them,
/// parents included. `insert_recursive` only adds an object plus what it directly
/// references (for a commit: its tree, recursively), NOT its ancestry, so a repository
/// with more than one commit produced a pack that made the client fail with
/// "Failed to traverse parents" / "remote did not send all necessary objects". The
/// reference implementation has the same defect. A revwalk over the tips, fed to
/// `insert_walk`, is the API that yields a complete pack.
fn build_pack(repo: &git2::Repository, wants: &[String]) -> Result<Vec<u8>> {
    let mut buf = git2::Buf::new();
    {
        let mut pb = repo
            .packbuilder()
            .map_err(|e| ToolError::internal(format!("packbuilder failed: {e}")))?;

        let mut walk = repo
            .revwalk()
            .map_err(|e| ToolError::internal(format!("revwalk failed: {e}")))?;
        let mut pushed_any = false;
        for want in wants {
            // Unknown or unreachable wants are skipped, like the reference does.
            if let Ok(oid) = Oid::from_str(want) {
                if walk.push(oid).is_ok() {
                    pushed_any = true;
                } else {
                    // Not a commit (a tag or a bare tree can be wanted directly).
                    let _ = pb.insert_recursive(oid, None);
                }
            }
        }
        if pushed_any {
            pb.insert_walk(&mut walk)
                .map_err(|e| ToolError::internal(format!("pack walk failed: {e}")))?;
        }

        pb.write_buf(&mut buf)
            .map_err(|e| ToolError::internal(format!("pack write failed: {e}")))?;
    }
    let pack = buf.to_vec();
    if pack.is_empty() {
        return Ok(empty_pack());
    }
    Ok(pack)
}

/// The canonical empty packfile: `PACK`, version 2, zero objects, sha1 trailer.
/// The trailer is a constant because it is the sha1 of the 12 fixed header bytes
/// (`printf 'PACK\0\0\0\2\0\0\0\0' | shasum`).
pub fn empty_pack() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(b"PACK");
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(
        &hex::decode("029d08823bd8a8eab510ad6ac75c823cfd3ed31e").expect("static hex"),
    );
    out
}

// ── receive-pack ────────────────────────────────────────────────────────────

/// One `old new refname` command from a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub old_sha: String,
    pub new_sha: String,
    pub ref_name: String,
}

/// Parse the command section, returning the updates, the client capability line and the
/// trailing packfile. The capabilities matter: when the client asked for `side-band-64k`
/// the report MUST be wrapped in band 1, otherwise git rejects it with "bad band".
pub fn parse_receive_pack_request(body: &[u8]) -> (Vec<RefUpdate>, String, &[u8]) {
    let mut updates = Vec::new();
    let mut client_caps = String::new();
    let mut reader = PktReader::new(body);
    let mut first = true;
    loop {
        let (line, kind) = reader.read_line();
        if kind == PacketType::Flush {
            break;
        }
        let Some(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        // Only the first command carries NUL separated client capabilities.
        let actual = if first && line.contains('\0') {
            let mut it = line.splitn(2, '\0');
            let cmd = it.next().unwrap_or("").to_string();
            client_caps = it.next().unwrap_or("").trim().to_string();
            cmd
        } else {
            line.clone()
        };
        first = false;

        let parts: Vec<&str> = actual.trim().splitn(3, ' ').collect();
        if parts.len() < 3 {
            continue;
        }
        updates.push(RefUpdate {
            old_sha: parts[0].to_string(),
            new_sha: parts[1].to_string(),
            ref_name: parts[2].to_string(),
        });
    }
    (updates, client_caps, reader.remaining())
}

/// Serve a push. The caller must already hold the project write lock.
async fn receive_pack_locked(
    entry: &GitRepoEntry,
    updates: &[RefUpdate],
    pack: &[u8],
    side_band: bool,
) -> Result<Vec<u8>> {
    // 12 bytes is the minimal pack header (magic, version, object count).
    if pack.len() > 12 {
        let repo = entry.repo.lock().await;
        index_pack(&repo, pack)?;
        entry.objects.import_from_repo(&repo).await?;
    }

    // `report-status` requires an `unpack` status line before the per ref lines.
    // The C# omits it, which makes a real `git push` report a failure even though the
    // refs update correctly, so this is a deliberate, documented divergence: a client
    // that cannot push is worse than a byte identical response (see module docs).
    let mut report = Vec::new();
    report.extend_from_slice(&pktline::encode("unpack ok\n"));
    for u in updates {
        let line = match process_ref_update(entry, u).await {
            Ok(l) => l,
            Err(e) => format!("ng {} {}", u.ref_name, e.message),
        };
        report.extend_from_slice(&pktline::encode(&format!("{line}\n")));
    }
    report.extend_from_slice(&pktline::flush());

    if !side_band {
        return Ok(report);
    }
    // The client asked for side-band-64k, so the whole report travels on band 1.
    // Without this git aborts with "protocol error: bad band #117" ('u' of "unpack").
    let mut out = Vec::new();
    for chunk in report.chunks(SIDE_BAND_CHUNK) {
        let mut framed = Vec::with_capacity(chunk.len() + 1);
        framed.push(1);
        framed.extend_from_slice(chunk);
        out.extend_from_slice(&pktline::encode_raw(&framed));
    }
    out.extend_from_slice(&pktline::flush());
    Ok(out)
}

/// Full push handling: parse, take the write lock, index, update refs, report.
pub async fn handle_receive_pack(entry: &GitRepoEntry, body: &[u8]) -> Result<Vec<u8>> {
    let (updates, client_caps, pack) = parse_receive_pack_request(body);
    if updates.is_empty() {
        return Ok(pktline::flush());
    }
    let side_band = client_caps.split_whitespace().any(|c| c == "side-band-64k");
    let _guard = entry.write_lock.lock().await;
    receive_pack_locked(entry, &updates, pack, side_band).await
}

fn index_pack(repo: &git2::Repository, pack: &[u8]) -> Result<()> {
    use std::io::Write;
    let odb = repo
        .odb()
        .map_err(|e| ToolError::internal(format!("odb open failed: {e}")))?;
    let mut writer = odb
        .packwriter()
        .map_err(|e| ToolError::internal(format!("packwriter failed: {e}")))?;
    writer
        .write_all(pack)
        .map_err(|e| ToolError::internal(format!("pack write failed: {e}")))?;
    writer
        .commit()
        .map_err(|e| ToolError::internal(format!("pack index failed: {e}")))?;
    Ok(())
}

/// Apply one command, returning the report-status line for it.
async fn process_ref_update(entry: &GitRepoEntry, u: &RefUpdate) -> Result<String> {
    if u.new_sha == ZERO_ID {
        entry.db.delete_ref(&u.ref_name).await?;
        return Ok(format!("ok {}", u.ref_name));
    }

    if !object_present(entry, &u.new_sha).await? {
        return Ok(format!("ng {} object not found", u.ref_name));
    }

    // Fast forward check: the client tells us what it thinks the old value is.
    if u.old_sha != ZERO_ID
        && let Some(existing) = entry.db.get_ref(&u.ref_name).await?
        && existing.target != u.old_sha
    {
        return Ok(format!("ng {} non-fast-forward", u.ref_name));
    }

    entry.db.set_ref(&u.ref_name, &u.new_sha, false).await?;
    // HEAD stays symbolic, so a branch update needs nothing extra there.
    Ok(format!("ok {}", u.ref_name))
}

async fn object_present(entry: &GitRepoEntry, sha: &str) -> Result<bool> {
    if entry.objects.exists(sha).await? {
        return Ok(true);
    }
    let Ok(oid) = Oid::from_str(sha) else {
        return Ok(false);
    };
    let repo = entry.repo.lock().await;
    Ok(repo.odb().map(|o| o.exists(oid)).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::git::odb::seed_commit;
    use git2::ObjectType;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    fn config(root: &std::path::Path) -> Arc<ServerConfig> {
        let mut c = ServerConfig::default();
        c.infra.meta.dir = root.join("state/volumes").display().to_string();
        c.infra.blob.dir = root.join("state/blobs").display().to_string();
        c.infra.admin.path = root.join("state/admin.db").display().to_string();
        c.git.enabled = true;
        Arc::new(c)
    }

    async fn entry(root: &std::path::Path) -> (Arc<GitRepoStore>, Arc<GitRepoEntry>) {
        let store = Arc::new(GitRepoStore::new(config(root)));
        let e = store.init_repo("proj").await.unwrap();
        (store, e)
    }

    fn read_pkt_lines(body: &[u8]) -> Vec<Option<String>> {
        let mut r = PktReader::new(body);
        let mut out = Vec::new();
        loop {
            let before = r.position();
            let (line, kind) = r.read_line();
            match kind {
                PacketType::Data => out.push(line),
                _ => out.push(None),
            }
            if r.position() == before {
                break;
            }
            if r.remaining().is_empty() {
                break;
            }
        }
        out
    }

    #[test]
    fn capability_strings_match_the_csharp() {
        assert_eq!(
            UPLOAD_PACK_CAPABILITIES,
            "multi_ack multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0"
        );
        assert_eq!(
            RECEIVE_PACK_CAPABILITIES,
            "report-status delete-refs side-band-64k quiet atomic ofs-delta agent=mcp-fs/0.1.0"
        );
        assert_eq!(ZERO_ID.len(), 40);
    }

    #[test]
    fn empty_pack_is_the_canonical_32_bytes() {
        let p = empty_pack();
        assert_eq!(p.len(), 32);
        assert_eq!(&p[..4], b"PACK");
        assert_eq!(&p[4..8], &[0, 0, 0, 2], "pack version 2");
        assert_eq!(&p[8..12], &[0, 0, 0, 0], "zero objects");
        assert_eq!(hex::encode(&p[12..]), "029d08823bd8a8eab510ad6ac75c823cfd3ed31e");
    }

    #[tokio::test]
    async fn empty_repo_advertises_capabilities_against_the_zero_id() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let body = advertise_upload_pack(&e).await.unwrap();

        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.starts_with("001e# service=git-upload-pack\n0000"));
        assert!(text.contains(&format!("{ZERO_ID} capabilities^{{}}\0")));
        assert!(text.contains("agent=mcp-fs/0.1.0"));
        assert!(text.ends_with("0000"));
    }

    #[tokio::test]
    async fn advertisement_lists_refs_with_capabilities_on_the_first_line_only() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let main = "1".repeat(40);
        let tag = "2".repeat(40);
        e.db.set_ref("refs/heads/main", &main, false).await.unwrap();
        e.db.set_ref("refs/tags/v1", &tag, false).await.unwrap();

        let body = advertise_upload_pack(&e).await.unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        // HEAD resolves through the symbolic ref and comes first
        assert!(text.contains(&format!("{main} HEAD\0{UPLOAD_PACK_CAPABILITIES}\n")));
        assert!(text.contains(&format!("{main} refs/heads/main\n")));
        assert!(text.contains(&format!("{tag} refs/tags/v1\n")));
        assert_eq!(
            text.matches("agent=mcp-fs/0.1.0").count(),
            1,
            "capabilities appear once, on the first ref line"
        );
    }

    #[tokio::test]
    async fn receive_pack_advertisement_omits_head() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let main = "3".repeat(40);
        e.db.set_ref("refs/heads/main", &main, false).await.unwrap();

        let body = advertise_receive_pack(&e).await.unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.starts_with("001f# service=git-receive-pack\n0000"));
        assert!(text.contains("report-status delete-refs"));
        assert!(!text.contains(" HEAD\0"), "receive-pack advertises branches only");
    }

    #[tokio::test]
    async fn concrete_refs_skips_unresolvable_symbolic_head() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        // HEAD points at a branch that does not exist yet (fresh repo)
        let refs = concrete_refs(&e, true).await.unwrap();
        assert!(refs.is_empty(), "an unborn HEAD advertises nothing");
    }

    #[test]
    fn parses_wants_and_haves() {
        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode(
            "want 1111111111111111111111111111111111111111 multi_ack side-band-64k\n",
        ));
        body.extend_from_slice(&pktline::encode(
            "want 2222222222222222222222222222222222222222\n",
        ));
        body.extend_from_slice(&pktline::encode(
            "have 3333333333333333333333333333333333333333\n",
        ));
        body.extend_from_slice(&pktline::flush());
        body.extend_from_slice(&pktline::encode("done\n"));

        let req = parse_upload_pack_request(&body);
        assert_eq!(req.wants, vec!["1".repeat(40), "2".repeat(40)]);
        assert_eq!(req.haves, vec!["3".repeat(40)]);
    }

    #[test]
    fn parses_an_empty_upload_pack_request() {
        let req = parse_upload_pack_request(&pktline::flush());
        assert!(req.wants.is_empty() && req.haves.is_empty());
    }

    #[tokio::test]
    async fn upload_pack_without_wants_answers_a_bare_flush() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let out = handle_upload_pack(&e, &pktline::flush()).await.unwrap();
        assert_eq!(out, b"0000");
    }

    #[tokio::test]
    async fn upload_pack_streams_a_pack_over_side_band() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let (commit, _tree, _blob) =
            seed_commit(&e.objects, "a.txt", b"hello from mcp-fs", "initial").await.unwrap();
        e.db.set_ref("refs/heads/main", &commit, false).await.unwrap();

        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode(&format!(
            "want {commit} multi_ack side-band-64k\n"
        )));
        body.extend_from_slice(&pktline::flush());
        body.extend_from_slice(&pktline::encode("done\n"));

        let out = handle_upload_pack(&e, &body).await.unwrap();
        assert!(out.starts_with(b"0008NAK\n"), "negotiation always NAKs");
        assert!(out.ends_with(b"0005\x010000"), "side-band close then flush");

        // reassemble channel 1 and check it is a real packfile
        let mut r = PktReader::new(&out);
        let mut pack = Vec::new();
        loop {
            let (data, kind) = r.read_packet();
            if kind != PacketType::Data {
                break;
            }
            let Some(data) = data else { break };
            if data == b"NAK\n" || data.len() <= 1 {
                continue;
            }
            assert_eq!(data[0], 1, "packfile rides channel 1");
            pack.extend_from_slice(&data[1..]);
        }
        assert!(pack.starts_with(b"PACK"), "pack magic present");
        assert!(pack.len() > 32, "pack carries the commit, tree and blob");
    }

    #[test]
    fn parses_ref_updates_and_splits_off_the_pack() {
        let old = "0".repeat(40);
        let new = "a".repeat(40);
        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode_raw(
            format!("{old} {new} refs/heads/main\0report-status side-band-64k\n").as_bytes(),
        ));
        body.extend_from_slice(&pktline::encode(&format!(
            "{new} {old} refs/heads/gone\n"
        )));
        body.extend_from_slice(&pktline::flush());
        body.extend_from_slice(b"PACKpayload");

        let (updates, caps, pack) = parse_receive_pack_request(&body);
        assert_eq!(updates.len(), 2);
        assert_eq!(
            caps, "report-status side-band-64k",
            "client capabilities are captured, not just discarded"
        );
        assert_eq!(
            updates[0],
            RefUpdate {
                old_sha: old.clone(),
                new_sha: new.clone(),
                ref_name: "refs/heads/main".into()
            },
            "capabilities must be stripped from the first command"
        );
        assert_eq!(updates[1].ref_name, "refs/heads/gone");
        assert_eq!(pack, b"PACKpayload");
    }

    #[tokio::test]
    async fn receive_pack_without_commands_answers_a_bare_flush() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let out = handle_receive_pack(&e, &pktline::flush()).await.unwrap();
        assert_eq!(out, b"0000");
    }

    /// A client that asked for side-band-64k must get the report on band 1. Without the
    /// wrapping, git aborts with "protocol error: bad band #117" (the 'u' of "unpack").
    #[tokio::test]
    async fn receive_pack_report_is_side_band_framed_when_requested() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let (commit, _t, _b) = seed_commit(&e.objects, "a.txt", b"x", "c1").await.unwrap();

        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode_raw(
            format!("{ZERO_ID} {commit} refs/heads/sb\0report-status side-band-64k\n").as_bytes(),
        ));
        body.extend_from_slice(&pktline::flush());
        let out = handle_receive_pack(&e, &body).await.unwrap();

        // Every data packet carries the band byte first.
        let mut reader = pktline::PktReader::new(&out);
        let (line, kind) = reader.read_line();
        assert_eq!(kind, PacketType::Data, "expected a data packet");
        let raw = line.expect("payload");
        assert!(raw.starts_with('\u{1}'), "report must be on band 1, got {raw:?}");
        assert!(raw.contains("unpack ok"), "band 1 carries the report: {raw:?}");
    }

    /// Without side-band-64k the report stays raw pkt-lines.
    #[tokio::test]
    async fn receive_pack_report_is_raw_without_side_band() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let (commit, _t, _b) = seed_commit(&e.objects, "a.txt", b"x", "c1").await.unwrap();

        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode_raw(
            format!("{ZERO_ID} {commit} refs/heads/raw\0report-status\n").as_bytes(),
        ));
        body.extend_from_slice(&pktline::flush());
        let out = handle_receive_pack(&e, &body).await.unwrap();
        let lines = read_pkt_lines(&out);
        assert_eq!(lines[0].as_deref(), Some("unpack ok"));
        assert_eq!(lines[1].as_deref(), Some("ok refs/heads/raw"));
    }

    #[tokio::test]
    async fn receive_pack_creates_a_ref_and_reports_ok() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let (commit, _t, _b) = seed_commit(&e.objects, "a.txt", b"x", "c1").await.unwrap();

        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode_raw(
            format!("{ZERO_ID} {commit} refs/heads/main\0report-status\n").as_bytes(),
        ));
        body.extend_from_slice(&pktline::flush());

        let out = handle_receive_pack(&e, &body).await.unwrap();
        let lines = read_pkt_lines(&out);
        // report-status starts with the unpack status, then one line per ref.
        assert_eq!(lines[0].as_deref(), Some("unpack ok"));
        assert_eq!(lines[1].as_deref(), Some("ok refs/heads/main"));
        assert_eq!(
            e.db.get_ref("refs/heads/main").await.unwrap().unwrap().target,
            commit
        );
    }

    #[tokio::test]
    async fn receive_pack_rejects_unknown_objects_and_non_fast_forward() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;

        // unknown object
        let ghost = "b".repeat(40);
        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode(&format!(
            "{ZERO_ID} {ghost} refs/heads/main\n"
        )));
        body.extend_from_slice(&pktline::flush());
        let out = handle_receive_pack(&e, &body).await.unwrap();
        let lines = read_pkt_lines(&out);
        assert_eq!(lines[0].as_deref(), Some("unpack ok"));
        assert_eq!(lines[1].as_deref(), Some("ng refs/heads/main object not found"));
        assert!(e.db.get_ref("refs/heads/main").await.unwrap().is_none());

        // non fast forward: client's old sha disagrees with the stored ref
        let (c1, _, _) = seed_commit(&e.objects, "a.txt", b"1", "c1").await.unwrap();
        let (c2, _, _) = seed_commit(&e.objects, "a.txt", b"2", "c2").await.unwrap();
        e.db.set_ref("refs/heads/main", &c1, false).await.unwrap();
        let wrong_old = "c".repeat(40);
        let mut body2 = Vec::new();
        body2.extend_from_slice(&pktline::encode(&format!(
            "{wrong_old} {c2} refs/heads/main\n"
        )));
        body2.extend_from_slice(&pktline::flush());
        let out2 = handle_receive_pack(&e, &body2).await.unwrap();
        let lines2 = read_pkt_lines(&out2);
        assert_eq!(lines2[0].as_deref(), Some("unpack ok"));
        assert_eq!(lines2[1].as_deref(), Some("ng refs/heads/main non-fast-forward"));
        assert_eq!(
            e.db.get_ref("refs/heads/main").await.unwrap().unwrap().target,
            c1,
            "a rejected update must not move the ref"
        );
    }

    #[tokio::test]
    async fn receive_pack_deletes_a_ref_on_zero_new_sha() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let (c1, _, _) = seed_commit(&e.objects, "a.txt", b"1", "c1").await.unwrap();
        e.db.set_ref("refs/heads/doomed", &c1, false).await.unwrap();

        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode(&format!(
            "{c1} {ZERO_ID} refs/heads/doomed\n"
        )));
        body.extend_from_slice(&pktline::flush());
        let out = handle_receive_pack(&e, &body).await.unwrap();
        let lines = read_pkt_lines(&out);
        assert_eq!(lines[0].as_deref(), Some("unpack ok"));
        assert_eq!(lines[1].as_deref(), Some("ok refs/heads/doomed"));
        assert!(e.db.get_ref("refs/heads/doomed").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn receive_pack_indexes_a_real_packfile_into_the_blob_store() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;

        // Build a pack the way a client would, from a scratch repo.
        let src_dir = tempfile::tempdir().unwrap();
        let src = git2::Repository::init_bare(src_dir.path()).unwrap();
        let blob = src.blob(b"pushed content").unwrap();
        let mut tb = src.treebuilder(None).unwrap();
        tb.insert("f.txt", blob, 0o100644).unwrap();
        let tree = tb.write().unwrap();
        let sig = git2::Signature::new("t", "t@example.test", &git2::Time::new(1700000000, 0))
            .unwrap();
        let commit = src
            .commit(None, &sig, &sig, "pushed", &src.find_tree(tree).unwrap(), &[])
            .unwrap();
        let mut buf = git2::Buf::new();
        {
            let mut pb = src.packbuilder().unwrap();
            pb.insert_recursive(commit, None).unwrap();
            pb.write_buf(&mut buf).unwrap();
        }

        let mut body = Vec::new();
        body.extend_from_slice(&pktline::encode_raw(
            format!("{ZERO_ID} {commit} refs/heads/main\0report-status\n").as_bytes(),
        ));
        body.extend_from_slice(&pktline::flush());
        body.extend_from_slice(&buf);

        let out = handle_receive_pack(&e, &body).await.unwrap();
        let lines = read_pkt_lines(&out);
        // report-status starts with the unpack status, then one line per ref.
        assert_eq!(lines[0].as_deref(), Some("unpack ok"));
        assert_eq!(lines[1].as_deref(), Some("ok refs/heads/main"));

        // every pushed object landed in the blob store under git:{sha}
        assert!(e.objects.exists(&commit.to_string()).await.unwrap());
        assert!(e.objects.exists(&blob.to_string()).await.unwrap());
        assert!(e.objects.exists(&tree.to_string()).await.unwrap());
        let (kind, payload) = e.objects.read(&blob.to_string()).await.unwrap();
        assert_eq!(kind, ObjectType::Blob);
        assert_eq!(payload, b"pushed content");
        assert!(e.objects.list().await.unwrap().len() >= 3);
    }

    // ── router level tests (auth gating, headers, status codes) ─────────────

    /// A throwaway RSA keypair written to disk plus a token signed with it.
    fn keypair_and_token(root: &std::path::Path, person: &str) -> (std::path::PathBuf, String) {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::pkcs8::EncodePublicKey;

        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_pem = priv_key
            .to_public_key()
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let priv_pem = priv_key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap();

        let pub_path = root.join("jwt-public.pem");
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(&pub_path, pub_pem).unwrap();

        let claims = serde_json::json!({
            "email": person,
            "iss": "web-a2a",
            "exp": chrono::Utc::now().timestamp() + 3600,
        });
        let token = encode(
            &Header::new(Algorithm::RS256),
            &claims,
            &EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap(),
        )
        .unwrap();
        (pub_path, token)
    }

    /// Router plus a token for `owner@test.com`, who is a member of `proj`.
    /// `initialized` controls whether the git repo exists yet.
    async fn harness(
        root: &std::path::Path,
        anonymous_read: bool,
        initialized: bool,
        max_pack_mb: u32,
    ) -> (Router, String) {
        let (pub_path, token) = keypair_and_token(root, "owner@test.com");
        let mut c = (*config(root)).clone();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        c.git.anonymous_read = anonymous_read;
        c.git.max_pack_size_mb = max_pack_mb;
        let config = Arc::new(c);

        let admin = crate::storage::build_admin_store(&config).unwrap();
        admin.connect().await.unwrap();
        admin.create_project("proj", "owner@test.com").await.unwrap();

        let app = Arc::new(crate::state::AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(config.clone())),
            safety: Arc::new(crate::safety::SafetyManager::new(config.safety.clone())),
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            registry: Arc::new(crate::mcp::ToolRegistry::new()),
        });
        let git = Arc::new(GitRepoStore::new(config));
        if initialized {
            git.init_repo("proj").await.unwrap();
        }
        (router(app, git), token)
    }

    fn get(uri: &str, token: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri(uri);
        if let Some(t) = token {
            b = b.header("X-Forwarded-Authorization", format!("Bearer {t}"));
        }
        b.body(Body::empty()).unwrap()
    }

    fn post(uri: &str, token: Option<&str>, body: Vec<u8>) -> Request<Body> {
        let mut b = Request::builder().method("POST").uri(uri);
        if let Some(t) = token {
            b = b.header("X-Forwarded-Authorization", format!("Bearer {t}"));
        }
        b.body(Body::from(body)).unwrap()
    }

    async fn body_bytes(r: Response) -> Vec<u8> {
        to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn info_refs_requires_a_known_service() {
        let d = tempfile::tempdir().unwrap();
        let (app, token) = harness(d.path(), false, true, 512).await;
        for uri in ["/git/proj/info/refs", "/git/proj/info/refs?service=git-nope"] {
            let r = app.clone().oneshot(get(uri, Some(&token))).await.unwrap();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST);
            assert_eq!(body_bytes(r).await, b"service parameter required");
        }
    }

    #[tokio::test]
    async fn unauthenticated_is_401_with_a_bearer_challenge() {
        let d = tempfile::tempdir().unwrap();
        let (app, _t) = harness(d.path(), false, true, 512).await;
        let r = app
            .clone()
            .oneshot(get("/git/proj/info/refs?service=git-upload-pack", None))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(r.headers()[header::WWW_AUTHENTICATE], "Bearer realm=\"mcp-fs\"");
        assert_eq!(body_bytes(r).await, b"authentication required");
    }

    #[tokio::test]
    async fn anonymous_read_allows_fetch_but_never_push() {
        let d = tempfile::tempdir().unwrap();
        let (app, _t) = harness(d.path(), true, true, 512).await;

        let r = app
            .clone()
            .oneshot(get("/git/proj/info/refs?service=git-upload-pack", None))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);

        // receive-pack advertisement and push both still need an identity
        let r = app
            .clone()
            .oneshot(get("/git/proj/info/refs?service=git-receive-pack", None))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

        let r = app
            .clone()
            .oneshot(post("/git/proj/git-receive-pack", None, pktline::flush()))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn non_members_are_forbidden_and_unknown_projects_are_404() {
        let d = tempfile::tempdir().unwrap();
        let (pub_path, stranger) = keypair_and_token(&d.path().join("k2"), "stranger@test.com");
        let mut c = (*config(d.path())).clone();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        let config = Arc::new(c);

        let admin = crate::storage::build_admin_store(&config).unwrap();
        admin.connect().await.unwrap();
        admin.create_project("proj", "owner@test.com").await.unwrap();
        let app = Arc::new(crate::state::AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(config.clone())),
            safety: Arc::new(crate::safety::SafetyManager::new(config.safety.clone())),
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            registry: Arc::new(crate::mcp::ToolRegistry::new()),
        });
        let git = Arc::new(GitRepoStore::new(config));
        git.init_repo("proj").await.unwrap();
        let router = router(app, git);

        // A verified token is not enough: this is the hardening over the C#.
        let r = router
            .clone()
            .oneshot(get("/git/proj/info/refs?service=git-upload-pack", Some(&stranger)))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        let r = router
            .clone()
            .oneshot(get("/git/ghost/info/refs?service=git-upload-pack", Some(&stranger)))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_member_reaching_an_uninitialized_repo_gets_404() {
        let d = tempfile::tempdir().unwrap();
        let (app, token) = harness(d.path(), false, false, 512).await;
        let r = app
            .oneshot(get("/git/proj/info/refs?service=git-upload-pack", Some(&token)))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_bytes(r).await, b"repository 'proj' not found");
    }

    #[tokio::test]
    async fn advertisement_carries_the_git_content_type_and_no_cache_headers() {
        let d = tempfile::tempdir().unwrap();
        let (app, token) = harness(d.path(), false, true, 512).await;

        let r = app
            .clone()
            .oneshot(get("/git/proj/info/refs?service=git-upload-pack", Some(&token)))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            r.headers()[header::CONTENT_TYPE],
            "application/x-git-upload-pack-advertisement"
        );
        assert_eq!(r.headers()[header::EXPIRES], "Fri, 01 Jan 1980 00:00:00 GMT");
        assert_eq!(r.headers()[header::PRAGMA], "no-cache");
        assert_eq!(
            r.headers()[header::CACHE_CONTROL],
            "no-cache, max-age=0, must-revalidate"
        );
        let body = String::from_utf8(body_bytes(r).await).unwrap();
        assert!(body.starts_with("001e# service=git-upload-pack\n0000"));

        let r = app
            .oneshot(get("/git/proj/info/refs?service=git-receive-pack", Some(&token)))
            .await
            .unwrap();
        assert_eq!(
            r.headers()[header::CONTENT_TYPE],
            "application/x-git-receive-pack-advertisement"
        );
    }

    #[tokio::test]
    async fn post_endpoints_serve_the_protocol_end_to_end() {
        let d = tempfile::tempdir().unwrap();
        let (app, token) = harness(d.path(), false, true, 512).await;

        let r = app
            .clone()
            .oneshot(post("/git/proj/git-upload-pack", Some(&token), pktline::flush()))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            r.headers()[header::CONTENT_TYPE],
            "application/x-git-upload-pack-result"
        );
        assert_eq!(body_bytes(r).await, b"0000", "no wants means a bare flush");

        let r = app
            .oneshot(post("/git/proj/git-receive-pack", Some(&token), pktline::flush()))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            r.headers()[header::CONTENT_TYPE],
            "application/x-git-receive-pack-result"
        );
    }

    #[tokio::test]
    async fn oversized_pushes_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        // 1 MB cap, 2 MB body
        let (app, token) = harness(d.path(), false, true, 1).await;
        let big = vec![b'x'; 2 * 1024 * 1024];
        let r = app
            .oneshot(post("/git/proj/git-receive-pack", Some(&token), big))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn unknown_routes_are_not_claimed_by_the_git_router() {
        let d = tempfile::tempdir().unwrap();
        let (app, token) = harness(d.path(), false, true, 512).await;
        let r = app.oneshot(get("/git/proj/objects/info/packs", Some(&token))).await.unwrap();
        assert_eq!(
            r.status(),
            StatusCode::NOT_FOUND,
            "the dumb protocol is deliberately not served"
        );
    }

    #[tokio::test]
    async fn a_push_then_a_fetch_round_trips_the_same_objects() {
        let d = tempfile::tempdir().unwrap();
        let (_s, e) = entry(d.path()).await;
        let (commit, _t, blob) = seed_commit(&e.objects, "a.txt", b"round trip", "c1").await.unwrap();
        e.db.set_ref("refs/heads/main", &commit, false).await.unwrap();

        let mut req = Vec::new();
        req.extend_from_slice(&pktline::encode(&format!("want {commit}\n")));
        req.extend_from_slice(&pktline::flush());
        let out = handle_upload_pack(&e, &req).await.unwrap();

        // the fetched pack must be indexable by a fresh client repo
        let mut pack = Vec::new();
        let mut r = PktReader::new(&out);
        while let (Some(data), PacketType::Data) = r.read_packet() {
            if data.len() > 1 && data[0] == 1 {
                pack.extend_from_slice(&data[1..]);
            }
        }
        let client_dir = tempfile::tempdir().unwrap();
        let client = git2::Repository::init_bare(client_dir.path()).unwrap();
        {
            use std::io::Write;
            let odb = client.odb().unwrap();
            let mut w = odb.packwriter().unwrap();
            w.write_all(&pack).unwrap();
            w.commit().unwrap();
        }
        assert!(client.odb().unwrap().exists(Oid::from_str(&commit).unwrap()));
        assert!(client.odb().unwrap().exists(Oid::from_str(&blob).unwrap()));
    }
}

