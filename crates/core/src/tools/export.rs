//! `fs.export_zip` (SPEC-0012 US-0002): package a selection of files and
//! directories into a zip, park it in the blob store, and hand back a single
//! use download URL.
//!
//! Typed function only: the `#[tool]` method in `mcp::server` and the REST
//! route `POST /api/fs/{mount_id}/export-zip` both authorize, then call
//! [`export_zip`], so the two doors share one implementation.

use crate::errors::{Result, ToolError};
use crate::state::AppState;
#[cfg(test)]
use crate::tools::authorize_only;
#[cfg(test)]
use crate::tools::registry_support::{ToolRegistry, ToolSchema, handler};
use serde_json::{Value, json};
use std::io::Write as _;

/// Lifetime of an export link (DEC-002: fixed, never caller configurable).
pub(crate) const EXPORT_TTL_SECS: i64 = 300;

/// Test-dispatch glue for `fs.export_zip`, mirroring the production handler
/// (`McpServer::fs_export_zip`) so the golden contract sees the same schema.
#[cfg(test)]
pub(crate) fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new(
            "fs.export_zip",
            "Zip a selection of files and directories and return a single-use download URL valid for 5 minutes.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str_array(
            "paths",
            "Absolute POSIX paths of files or directories to include; directories are walked recursively.",
        )
        .read_only(false)
        .destructive(false)
        .idempotent(false)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let paths = a.req_str_array("paths")?;
            export_zip(&ctx.state, &mount, &paths).await
        }),
    );
}

/// `fs.export_zip(mount_id, paths)` (FR-NEW-001..008). The caller has already
/// authorized `mount_id` (FR-NEW-004 runs strictly before this).
///
/// All or nothing: every path is validated and resolved before the first byte
/// is read, so a bad entry anywhere in the list creates neither blob nor row.
///
/// Time: O(n + B) where n is the number of nodes under the selection (one
/// subtree query per directory entry) and B the total bytes of the selected
/// files. Space: O(B) twice over at peak, the file bytes being compressed plus
/// the in memory archive, the same fully buffered model `download-zip` has
/// (DEC-010: no size or count limit).
pub(crate) async fn export_zip(
    state: &AppState,
    mount_id: &str,
    paths: &[String],
) -> Result<Value> {
    if paths.is_empty() {
        return Err(ToolError::invalid_argument("paths must name at least one file or directory"));
    }
    let mut normalized = Vec::with_capacity(paths.len());
    for raw in paths {
        // An empty entry would normalize to `/` and silently export the whole
        // volume, which is never what a caller who sent `""` meant.
        if raw.trim().is_empty() {
            return Err(ToolError::invalid_argument("paths must not contain an empty entry"));
        }
        ensure_no_escape(raw)?;
        normalized.push((raw.as_str(), state.safety.normalize_path(raw)?));
    }

    let client = state.stores.client(mount_id).await?;
    let files = resolve_files(&client, &normalized).await?;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for full in &files {
        let data = client.read_bytes(full).await?;
        zip.start_file(full.trim_start_matches('/'), options).map_err(zip_error)?;
        zip.write_all(&data).map_err(|e| ToolError::internal(e.to_string()))?;
    }
    let bytes = zip.finish().map_err(zip_error)?.into_inner();

    let token = uuid::Uuid::new_v4().to_string();
    let key = format!("export:{token}");
    client.blob.put(&key, &bytes).await?;
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::seconds(EXPORT_TTL_SECS);
    if let Err(e) = insert_link(state, mount_id, &token, &iso(now), &iso(expires)).await {
        // No row means nothing will ever sweep this blob, so drop it here.
        let _ = client.blob.delete(&key).await;
        return Err(e);
    }

    // Only a prefix: the full token is a bearer credential for the download.
    tracing::info!(mount_id, files = files.len(), token_prefix = &token[..8], "fs.export_zip");
    let base = state.config.server.public_base_url.trim_end_matches('/');
    Ok(json!({"url": format!("{base}/exports/{token}")}))
}

/// The RFC 3339 form every `export_links` timestamp is written in. One fixed
/// format matters: liveness is a string comparison in SQL, so two spellings of
/// the same instant would not compare correctly.
pub(crate) fn iso(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// Rejects a raw path whose `..` components climb above the volume root.
///
/// `normalize_path` follows posixpath and silently clamps `/../etc` to `/etc`;
/// for an export that would turn an escape attempt into a quiet read of some
/// other in volume path, so the climb itself is refused (FR-NEW-005).
fn ensure_no_escape(raw: &str) -> Result<()> {
    let mut depth: usize = 0;
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    ToolError::path_out_of_bounds(format!("path escapes the volume root: {raw}"))
                })?;
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

/// Expands the selection to the distinct files it names, in selection order:
/// a file stands for itself, a directory for every file under it. Every entry
/// is checked before anything is read, so one missing path fails the whole
/// request (FR-NEW-006). Overlapping entries are deduplicated, since a
/// repeated name would make the archive invalid.
async fn resolve_files(
    client: &crate::storage::VolumeClient,
    selection: &[(&str, String)],
) -> Result<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for (raw, norm) in selection {
        let node = client
            .meta
            .get(norm)
            .await?
            .ok_or_else(|| ToolError::not_found(format!("'{raw}' not found")))?;
        if !node.is_dir() {
            if seen.insert(norm.clone()) {
                files.push(norm.clone());
            }
            continue;
        }
        for (dirpath, _, names) in client.walk(norm).await? {
            for name in names {
                let full = format!("{}/{}", dirpath.trim_end_matches('/'), name);
                if seen.insert(full.clone()) {
                    files.push(full);
                }
            }
        }
    }
    Ok(files)
}

/// One `export_links` row, through US-0001's accessor, on the same relational
/// engine that holds this volume's metadata.
async fn insert_link(
    state: &AppState,
    mount_id: &str,
    token: &str,
    created_at: &str,
    expires_at: &str,
) -> Result<()> {
    let db =
        crate::storage::open_meta_db(&state.config, state.stores.relational(), mount_id).await?;
    let mut tx = db.begin().await?;
    crate::storage::meta::insert_export_link(&mut *tx, token, mount_id, created_at, expires_at)
        .await?;
    tx.commit().await
}

fn zip_error(e: zip::result::ZipError) -> ToolError {
    ToolError::internal(format!("zip: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::storage::VolumeClient;
    use crate::storage::traits::ExportLinkRow;
    use crate::tools::admin::test_support::Fixture;
    use std::io::Read as _;
    use std::sync::Arc;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "proj";

    fn p(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// The story's fixture volume: `proj`, owned by `OWNER`.
    async fn fixture_with(tweak: impl FnOnce(&mut crate::config::ServerConfig)) -> Fixture {
        let f = Fixture::with_config(tweak).await;
        f.seed_project(MOUNT, OWNER).await;
        let c = f.state.stores.client(MOUNT).await.unwrap();
        c.write_bytes_atomic("/src/main.rs", b"fn main() {}").await.unwrap();
        c.write_bytes_atomic("/docs/readme.md", b"# Hello").await.unwrap();
        c.write_bytes_atomic("/docs/notes/todo.txt", b"buy milk").await.unwrap();
        f
    }

    async fn fixture() -> Fixture {
        fixture_with(|_| {}).await
    }

    async fn client(f: &Fixture) -> Arc<VolumeClient> {
        f.state.stores.client(MOUNT).await.unwrap()
    }

    /// Every `export_links` row of the volume, live or not.
    async fn rows(f: &Fixture) -> Vec<ExportLinkRow> {
        let db = crate::storage::open_meta_db(&f.state.config, f.state.stores.relational(), MOUNT)
            .await
            .unwrap();
        let mut tx = db.begin().await.unwrap();
        let out = crate::storage::meta::select_expired_export_links(
            &mut *tx,
            MOUNT,
            "9999-12-31T23:59:59Z",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        out
    }

    /// Every blob file whose key carries the `export:` prefix, anywhere under
    /// the blob root.
    fn export_blobs(f: &Fixture) -> Vec<std::path::PathBuf> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if e.file_name().to_string_lossy().starts_with("export:") {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(std::path::Path::new(&f.state.config.infra.blob.dir), &mut out);
        out
    }

    async fn assert_nothing_created(f: &Fixture) {
        assert!(rows(f).await.is_empty(), "no export_links row may exist");
        assert!(export_blobs(f).is_empty(), "no export:* blob may exist");
    }

    fn token_of(url: &str) -> String {
        url.rsplit('/').next().unwrap().to_string()
    }

    fn is_relative_export_url(url: &str) -> bool {
        let Some(t) = url.strip_prefix("/exports/") else { return false };
        t.len() == 36 && t.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    }

    /// The archive parked at `export:{token}`, as `(entry name, bytes)` in order.
    async fn zip_entries(f: &Fixture, token: &str) -> Vec<(String, Vec<u8>)> {
        let bytes = client(f).await.blob.get(&format!("export:{token}"), 0, None).await.unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut e = archive.by_index(i).unwrap();
                let mut data = Vec::new();
                e.read_to_end(&mut data).unwrap();
                (e.name().to_string(), data)
            })
            .collect()
    }

    async fn export(f: &Fixture, list: &[&str]) -> Result<Value> {
        export_zip(&f.state, MOUNT, &p(list)).await
    }

    /// E2E-NEW-001: mixed file + directory selection, relative URL.
    #[tokio::test]
    async fn e2e_new_001_mixed_selection_returns_a_relative_url() {
        let f = fixture().await;
        let r = export(&f, &["/src/main.rs", "/docs"]).await.unwrap();
        let url = r["url"].as_str().unwrap();
        assert!(is_relative_export_url(url), "unexpected url {url}");
        let mut names: Vec<String> =
            zip_entries(&f, &token_of(url)).await.into_iter().map(|(n, _)| n).collect();
        names.sort();
        assert_eq!(names, ["docs/notes/todo.txt", "docs/readme.md", "src/main.rs"]);
    }

    /// E2E-NEW-003: an empty selection is `ERR_INVALID_ARGUMENT`, nothing created.
    #[tokio::test]
    async fn e2e_new_003_empty_paths_is_invalid_argument() {
        let f = fixture().await;
        let e = export(&f, &[]).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_nothing_created(&f).await;
    }

    /// E2E-NEW-005: one escaping path rejects the whole request.
    #[tokio::test]
    async fn e2e_new_005_escaping_path_rejects_everything() {
        let f = fixture().await;
        let e = export(&f, &["/src/main.rs", "/../../../etc/passwd"]).await.unwrap_err();
        assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS);
        assert_nothing_created(&f).await;
    }

    /// E2E-NEW-006: one missing file rejects the whole request, naming it.
    #[tokio::test]
    async fn e2e_new_006_missing_file_is_not_found() {
        let f = fixture().await;
        let e = export(&f, &["/src/main.rs", "/src/missing.rs"]).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(e.message.contains("/src/missing.rs"), "{}", e.message);
        assert_nothing_created(&f).await;
    }

    /// E2E-NEW-007: one missing directory rejects the whole request, naming it.
    #[tokio::test]
    async fn e2e_new_007_missing_directory_is_not_found() {
        let f = fixture().await;
        let e = export(&f, &["/docs", "/nonexistent-dir"]).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(e.message.contains("/nonexistent-dir"), "{}", e.message);
        assert_nothing_created(&f).await;
    }

    /// E2E-NEW-008: unicode and special characters survive as the entry name.
    #[tokio::test]
    async fn e2e_new_008_unicode_entry_name_round_trips() {
        let f = fixture().await;
        let path = "/docs/日本語 résumé (final)!.txt";
        client(&f).await.write_bytes_atomic(path, b"content").await.unwrap();
        let r = export(&f, &[path]).await.unwrap();
        let entries = zip_entries(&f, &token_of(r["url"].as_str().unwrap())).await;
        assert_eq!(entries, [("docs/日本語 résumé (final)!.txt".to_string(), b"content".to_vec())]);
    }

    /// E2E-NEW-009: traversal attempts, slash and backslash spelled.
    #[tokio::test]
    async fn e2e_new_009_traversal_attempts_are_out_of_bounds() {
        let f = fixture().await;
        for bad in ["/src/../../outside.txt", "..\\..\\windows\\system32"] {
            let e = export(&f, &[bad]).await.unwrap_err();
            assert_eq!(e.code, code::PATH_OUT_OF_BOUNDS, "{bad}");
        }
        assert_nothing_created(&f).await;
    }

    /// E2E-NEW-010: single file, empty directory, and a mixed selection.
    #[tokio::test]
    async fn e2e_new_010_single_file_empty_dir_and_mixed() {
        let f = fixture().await;
        let c = client(&f).await;
        c.write_bytes_atomic("/a.txt", b"A").await.unwrap();
        c.makedirs("/empty_dir", true).await.unwrap();
        c.write_bytes_atomic("/b/c.txt", b"C").await.unwrap();

        let a = export(&f, &["/a.txt"]).await.unwrap();
        let a = zip_entries(&f, &token_of(a["url"].as_str().unwrap())).await;
        assert_eq!(a, [("a.txt".to_string(), b"A".to_vec())]);

        let b = export(&f, &["/empty_dir"]).await.unwrap();
        assert!(zip_entries(&f, &token_of(b["url"].as_str().unwrap())).await.is_empty());

        let m = export(&f, &["/a.txt", "/b"]).await.unwrap();
        let m = zip_entries(&f, &token_of(m["url"].as_str().unwrap())).await;
        assert_eq!(
            m,
            [("a.txt".to_string(), b"A".to_vec()), ("b/c.txt".to_string(), b"C".to_vec())]
        );
    }

    /// An overlapping selection (a directory and a file inside it) yields each
    /// file once: a duplicate entry name would make the archive invalid.
    #[tokio::test]
    async fn overlapping_selection_yields_each_file_once() {
        let f = fixture().await;
        let r = export(&f, &["/docs", "/docs/readme.md"]).await.unwrap();
        let entries = zip_entries(&f, &token_of(r["url"].as_str().unwrap())).await;
        assert_eq!(entries.len(), 2, "{entries:?}");
    }

    /// E2E-NEW-011: one row with the token, the volume and a 300 s window.
    #[tokio::test]
    async fn e2e_new_011_row_carries_token_volume_and_window() {
        let f = fixture().await;
        let t0 = chrono::Utc::now();
        let r = export(&f, &["/src/main.rs"]).await.unwrap();
        let token = token_of(r["url"].as_str().unwrap());

        let rows = rows(&f).await;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.token, token);
        assert_eq!(row.volume_id, MOUNT);
        let created = chrono::DateTime::parse_from_rfc3339(&row.created_at).unwrap();
        let expires = chrono::DateTime::parse_from_rfc3339(&row.expires_at).unwrap();
        let since = (created.with_timezone(&chrono::Utc) - t0).num_milliseconds();
        assert!((0..=2000).contains(&since), "created_at {since} ms after t0");
        let window = (expires - created).num_milliseconds();
        assert!((299_000..=301_000).contains(&window), "window {window} ms");
    }

    /// E2E-NEW-012: the archive is stored at `export:{token}`.
    #[tokio::test]
    async fn e2e_new_012_archive_stored_at_export_token() {
        let f = fixture().await;
        let r = export(&f, &["/src/main.rs"]).await.unwrap();
        let entries = zip_entries(&f, &token_of(r["url"].as_str().unwrap())).await;
        assert_eq!(entries, [("src/main.rs".to_string(), b"fn main() {}".to_vec())]);
    }

    /// E2E-NEW-013: a configured `public_base_url` prefixes the URL.
    #[tokio::test]
    async fn e2e_new_013_public_base_url_prefixes_the_url() {
        let f =
            fixture_with(|c| c.server.public_base_url = "https://files.example.com".into()).await;
        let r = export(&f, &["/src/main.rs"]).await.unwrap();
        let url = r["url"].as_str().unwrap();
        let token = token_of(url);
        assert_eq!(url, format!("https://files.example.com/exports/{token}"));
    }

    /// E2E-NEW-014: the default empty `public_base_url` keeps the URL relative.
    #[tokio::test]
    async fn e2e_new_014_empty_public_base_url_keeps_url_relative() {
        let f = fixture().await;
        let r = export(&f, &["/src/main.rs"]).await.unwrap();
        let url = r["url"].as_str().unwrap();
        assert_eq!(url, format!("/exports/{}", token_of(url)));
        assert!(!url.starts_with("http"));
    }

    /// E2E-NEW-043: an empty string entry is an error, never silently dropped.
    #[tokio::test]
    async fn e2e_new_043_empty_string_entry_is_rejected() {
        let f = fixture().await;
        let e = export(&f, &["/src/main.rs", ""]).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_nothing_created(&f).await;
    }

    /// E2E-NEW-022: 10,000 seeded random bytes round trip byte for byte.
    #[tokio::test]
    async fn e2e_new_022_binary_content_round_trips_exactly() {
        use rand::{RngCore, SeedableRng};
        let f = fixture().await;
        let mut expected = vec![0u8; 10_000];
        rand::rngs::StdRng::seed_from_u64(42).fill_bytes(&mut expected);
        client(&f).await.write_bytes_atomic("/data/report.bin", &expected).await.unwrap();

        let r = export(&f, &["/data/report.bin"]).await.unwrap();
        let entries = zip_entries(&f, &token_of(r["url"].as_str().unwrap())).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "data/report.bin");
        assert!(entries[0].1 == expected, "bytes differ");
    }

    /// E2E-NEW-041: an unauthenticated `tools/call` of `fs.export_zip` gets
    /// exactly what `fs.read` gets under the same condition, cross checked
    /// through the real router rather than hardcoded, and stores nothing.
    #[tokio::test]
    async fn e2e_new_041_unauthenticated_call_matches_other_tools() {
        use tower::ServiceExt as _;
        let dir = tempfile::tempdir().unwrap();
        let (_key, pub_path) = crate::keys::write_keypair(dir.path().join("keys")).unwrap();
        let mut c = crate::config::ServerConfig::default();
        c.auth.jwt.public_key_path = pub_path.display().to_string();
        c.infra.meta.dir = dir.path().join("volumes").display().to_string();
        c.infra.blob.dir = dir.path().join("blobs").display().to_string();
        c.infra.admin.path = dir.path().join("admin.db").display().to_string();
        let app = crate::app::build(c).await.unwrap();

        let call = |tool: &str, args: Value| {
            let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": tool, "arguments": args}});
            axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("Host", "localhost")
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap()
        };
        let send = |req: axum::http::Request<axum::body::Body>| {
            let app = app.clone();
            async move {
                let r = app.oneshot(req).await.unwrap();
                let status = r.status();
                let b = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
                (status, serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null))
            }
        };

        let (s_export, b_export) =
            send(call("fs.export_zip", json!({"mount_id": MOUNT, "paths": ["/src/main.rs"]})))
                .await;
        let (s_read, b_read) =
            send(call("fs.read", json!({"mount_id": MOUNT, "path": "/src/main.rs"}))).await;
        assert_eq!(s_export, s_read);
        assert_eq!(b_export["error"], b_read["error"]);
        assert_eq!(b_export["error"], code::UNAUTHENTICATED);

        let mut stored = Vec::new();
        let mut stack = vec![dir.path().join("blobs")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    stored.push(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
        assert!(!stored.iter().any(|n| n.starts_with("export:")), "{stored:?}");
    }

    /// E2E-NEW-039 / E2E-NEW-053: no invalidation surface. The only tool name
    /// containing `export` is `fs.export_zip`, no `admin.*` tool names one, and
    /// no CLI verb is an export verb.
    #[test]
    fn e2e_new_039_053_no_export_invalidation_surface() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../TOOL_CONTRACT.txt"
        ))
        .unwrap();
        let tool_names: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with(' ') && !l.contains(':') && !l.trim().is_empty())
            .collect();
        let exporting: Vec<&&str> = tool_names.iter().filter(|n| n.contains("export")).collect();
        assert_eq!(exporting, [&"fs.export_zip"]);
        assert!(!tool_names.iter().any(|n| n.starts_with("admin.") && n.contains("export")));

        use clap::CommandFactory as _;
        let verbs: Vec<String> = crate::cli::Cli::command()
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        assert!(verbs.iter().all(|v| !v.contains("export")), "{verbs:?}");
    }
}
