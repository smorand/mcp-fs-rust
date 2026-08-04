//! Every filesystem operation, once. A 1:1 port of the C# `Core/FsOps.cs`:
//! same return keys, same caps, same error codes, same ordering of the safety
//! steps (normalize, read guard, quota, write, audit).
//!
//! The tool layer and the REST data plane both call these free functions, so the
//! two surfaces can never drift apart.

// Several operations mirror C# methods with 8+ parameters. Grouping them into
// structs would break the "tools layer calls them directly" contract, so the
// parameter lists are kept as-is on purpose.
#![allow(clippy::too_many_arguments)]

use crate::core::diff;
use crate::errors::{Result, ToolError};
use crate::safety::SafetyManager;
use crate::storage::VolumeClient;
use crate::util::text::split_lines;
use base64::Engine as _;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Directory names pruned from every recursive walk (glob, grep, symbol search).
pub const DEFAULT_EXCLUDES: &[&str] =
    &[".git", "node_modules", "target", "dist", ".build", "coverage", ".mcp_trash"];

/// Directory names pruned from `fs.tree`. Deliberately different from
/// `DEFAULT_EXCLUDES`: the C# implementation does not hide the trash dir here.
const TREE_EXCLUDES: &[&str] = &[".git", "node_modules", "target", "dist", ".build", "coverage"];

const ALLOWED_ALGOS: &[&str] = &["md5", "sha1", "sha256", "sha512"];

/// Hard ceiling on how many files one walk will visit.
const MAX_FILES: usize = 5000;
/// `fs.glob` returns at most this many paths.
const GLOB_CAP: usize = 100;
/// `fs.tree` stops after this many nodes and reports `truncated`.
const TREE_CAP: usize = 2000;
/// Minimum similarity for `search_replace(fuzzy=true)` to accept a block.
const FUZZY_THRESHOLD: f64 = 0.6;

/// Synthetic owner reported by `fs.stat`; the volume has no real POSIX owner.
const STAT_UID: i64 = 1000;
const STAT_GID: i64 = 1000;

// ─────────────────────────────────────────────────────────────────── read ─────

/// Paged, optionally line-numbered window over a text file.
/// Keys: `content`, `total_lines`, `truncated`, `next_offset`.
pub async fn read_window(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    offset_lines: i64,
    limit_lines: i64,
    line_numbered: bool,
) -> Result<Value> {
    let text = client.read_text(path).await?;
    safety.record_read(person, mount_id, path);
    let lines = split_lines(&text);
    let total = lines.len() as i64;
    let cap = limit_lines.min(safety.config().max_read_lines as i64);
    let window = slice(&lines, offset_lines, offset_lines.saturating_add(cap));
    let truncated = offset_lines.saturating_add(cap) < total;
    let content = if line_numbered {
        number_lines(window, offset_lines + 1)
    } else {
        window.join("\n")
    };
    Ok(json!({
        "content": content,
        "total_lines": total,
        "truncated": truncated,
        "next_offset": if truncated { json!(offset_lines + cap) } else { Value::Null },
    }))
}

/// Raw byte range as base64 plus the guessed MIME type.
/// Keys: `base64`, `mime_type`, `length`.
pub async fn read_bytes_b64(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    offset: i64,
    length: i64,
) -> Result<Value> {
    let data = client
        .read_range(path, offset.max(0) as u64, length.max(0) as u64)
        .await?;
    safety.record_read(person, mount_id, path);
    Ok(json!({
        "base64": base64::engine::general_purpose::STANDARD.encode(&data),
        "mime_type": mime_guess(path).unwrap_or("application/octet-stream"),
        "length": data.len(),
    }))
}

/// Line count without returning content. Key: `total_lines`.
pub async fn count_lines(client: &VolumeClient, path: &str) -> Result<Value> {
    let text = client.read_text(path).await?;
    Ok(json!({ "total_lines": split_lines(&text).len() }))
}

/// Inclusive 1-based line range. Keys: `content`, `total_lines`.
pub async fn read_lines(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    start_line: i64,
    end_line: i64,
) -> Result<Value> {
    let text = client.read_text(path).await?;
    safety.record_read(person, mount_id, path);
    let lines = split_lines(&text);
    let window = slice(&lines, (start_line - 1).max(0), end_line);
    Ok(json!({
        "content": number_lines(window, start_line.max(1)),
        "total_lines": lines.len(),
    }))
}

/// The indentation block surrounding an anchor line.
/// Keys: `content`, `start_line`, `end_line`.
pub async fn read_section(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    anchor_line: i64,
    max_lines: i64,
) -> Result<Value> {
    let text = client.read_text(path).await?;
    safety.record_read(person, mount_id, path);
    let lines = split_lines(&text);
    let (start, end) = indent_block(&lines, anchor_line - 1, max_lines)?;
    Ok(json!({
        "content": number_lines(slice(&lines, start as i64, end as i64), start as i64 + 1),
        "start_line": start + 1,
        "end_line": end,
    }))
}

/// Batch read with per-file error isolation: one bad path never fails the call.
/// Key: `files`, each entry `{path, content, truncated}` or `{path, error}`.
pub async fn read_many(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    paths: &[String],
    per_file_cap_lines: i64,
) -> Result<Value> {
    let mut results = Vec::with_capacity(paths.len());
    for raw in paths {
        // A rejected path keeps the raw spelling and the rendered "ERR_*: message",
        // which is what the C# ToolError.Message carries.
        let norm = match safety.normalize_path(raw) {
            Ok(n) => n,
            Err(e) => {
                results.push(json!({ "path": raw, "error": e.to_string() }));
                continue;
            }
        };
        let text = match client.read_text(&norm).await {
            Ok(t) => t,
            Err(e) => {
                results.push(json!({ "path": raw, "error": storage_error_text(&e, &norm) }));
                continue;
            }
        };
        safety.record_read(person, mount_id, &norm);
        let lines = split_lines(&text);
        results.push(json!({
            "path": norm,
            "content": number_lines(slice(&lines, 0, per_file_cap_lines), 1),
            "truncated": lines.len() as i64 > per_file_cap_lines,
        }));
    }
    Ok(json!({ "files": results }))
}

/// Per-file error text for `read_many`. The C# storage layer raises bare
/// `IOException`s here, so the string is the low-level `"not found: {path}"`
/// rather than an `ERR_*`-prefixed tool error.
fn storage_error_text(e: &ToolError, path: &str) -> String {
    match e.code {
        crate::errors::code::NOT_FOUND => format!("not found: {path}"),
        // Reading a directory is `NotFoundException` in C# (kind != "file") but
        // `ERR_INVALID_ARGUMENT` in the Rust storage layer; report it the C# way.
        crate::errors::code::INVALID_ARGUMENT if e.message.contains("is a directory") => {
            format!("not found: {path}")
        }
        _ => e.to_string(),
    }
}

/// First N lines. Key: `content`.
pub async fn head(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    lines: i64,
) -> Result<Value> {
    let text = client.read_text(path).await?;
    safety.record_read(person, mount_id, path);
    let all = split_lines(&text);
    Ok(json!({ "content": number_lines(slice(&all, 0, lines), 1) }))
}

/// Last N lines, numbered with their real line numbers. Key: `content`.
pub async fn tail(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    path: &str,
    lines: i64,
) -> Result<Value> {
    let text = client.read_text(path).await?;
    safety.record_read(person, mount_id, path);
    let all = split_lines(&text);
    let start = (all.len() as i64 - lines).max(0);
    Ok(json!({
        "content": number_lines(slice(&all, start, all.len() as i64), start + 1),
    }))
}

// ─────────────────────────────────────────────────────────────── metadata ─────

/// POSIX metadata. Keys: `path`, `size`, `mode`, `kind`, `mtime`, `ctime`,
/// `atime`, `uid`, `gid`.
pub async fn stat_info(client: &VolumeClient, path: &str) -> Result<Value> {
    let st = client.stat(path).await?;
    Ok(json!({
        "path": path,
        "size": st.size,
        "mode": oct_permissions(st.mode),
        "kind": kind_of(st.mode),
        "mtime": st.mtime,
        "ctime": st.ctime,
        "atime": st.atime,
        "uid": STAT_UID,
        "gid": STAT_GID,
    }))
}

/// Existence probe. Keys: `exists`, `kind` (null when absent).
pub async fn exists_info(client: &VolumeClient, path: &str) -> Result<Value> {
    if !client.exists(path).await? {
        return Ok(json!({ "exists": false, "kind": Value::Null }));
    }
    let st = client.stat(path).await?;
    Ok(json!({ "exists": true, "kind": kind_of(st.mode) }))
}

/// Content hash. Keys: `path`, `algo`, `hash`, `size`.
/// `ERR_INVALID_ARGUMENT` for anything outside md5|sha1|sha256|sha512.
pub async fn hash_file(client: &VolumeClient, path: &str, algo: &str) -> Result<Value> {
    if !ALLOWED_ALGOS.contains(&algo) {
        return Err(ToolError::invalid_argument(format!("unsupported algo '{algo}'")));
    }
    let data = client.read_bytes(path).await?;
    let hash = match algo {
        "md5" => hashing::md5_hex(&data),
        "sha1" => hashing::sha1_hex(&data),
        "sha512" => {
            use sha2::Digest;
            let mut h = sha2::Sha512::new();
            h.update(&data);
            hex::encode(h.finalize())
        }
        _ => VolumeClient::sha256_hex(&data),
    };
    Ok(json!({ "path": path, "algo": algo, "hash": hash, "size": data.len() }))
}

// ───────────────────────────────────────────────────────────────── search ─────

/// Walk `root` collecting `(path, mtime)` for every file, pruning any directory
/// whose path contains one of `excludes` as a segment. Capped at `MAX_FILES`.
pub async fn iter_files(
    client: &VolumeClient,
    root: &str,
    excludes: &[&str],
) -> Result<Vec<(String, f64)>> {
    let mut files: Vec<(String, f64)> = Vec::new();
    for (dirpath, _, filenames) in client.walk(root).await? {
        let with_slash = format!("{dirpath}/");
        if excludes
            .iter()
            .any(|seg| with_slash.contains(&format!("/{seg}")) || dirpath.ends_with(&format!("/{seg}")))
        {
            continue;
        }
        for filename in filenames {
            let full = format!("{}/{}", dirpath.trim_end_matches('/'), filename);
            // A racing delete just drops the entry, it never fails the walk.
            if let Ok(st) = client.stat(&full).await {
                files.push((full, st.mtime));
            } else {
                continue;
            }
            if files.len() >= MAX_FILES {
                return Ok(files);
            }
        }
    }
    Ok(files)
}

/// Glob over file paths, newest first, capped at 100.
/// Keys: `matches`, `truncated`.
pub async fn glob_files(
    client: &VolumeClient,
    root: &str,
    pattern: &str,
    extra_excludes: &[String],
) -> Result<Value> {
    let matcher = crate::util::text::Fnmatch::new(pattern);
    let exclude_matchers: Vec<crate::util::text::Fnmatch> = extra_excludes.iter().map(|g| crate::util::text::Fnmatch::new(g)).collect();
    let mut matched: Vec<(String, f64)> = Vec::new();
    for (path, mtime) in iter_files(client, root, DEFAULT_EXCLUDES).await? {
        let name = match path.rfind('/') {
            Some(i) => &path[i + 1..],
            None => path.as_str(),
        };
        if (matcher.is_match(&path) || matcher.is_match(name))
            && !exclude_matchers.iter().any(|m| m.is_match(&path))
        {
            matched.push((path, mtime));
        }
    }
    // Newest first. Stable so equal mtimes keep the walk order.
    matched.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(json!({
        "matches": matched.iter().take(GLOB_CAP).map(|m| m.0.clone()).collect::<Vec<_>>(),
        "truncated": matched.len() > GLOB_CAP,
    }))
}

/// Content search. `output_mode` selects the shape:
/// `files` -> `{files}`, `count` -> `{count, files}`, anything else ->
/// `{matches, truncated}`.
pub async fn grep_files(
    client: &VolumeClient,
    root: &str,
    pattern: &str,
    include_glob: Option<&str>,
    exclude_glob: Option<&str>,
    regex: bool,
    case_sensitive: bool,
    output_mode: &str,
    context_lines: i64,
    max_matches: i64,
) -> Result<Value> {
    let source = if regex { pattern.to_string() } else { regex::escape(pattern) };
    let matcher = regex::RegexBuilder::new(&source)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| ToolError::invalid_argument(format!("invalid pattern '{pattern}': {e}")))?;
    let include = include_glob.map(crate::util::text::Fnmatch::new);
    let exclude = exclude_glob.map(crate::util::text::Fnmatch::new);

    let mut hits: Vec<Value> = Vec::new();
    let mut files_with_matches: Vec<String> = Vec::new();
    for (path, _) in iter_files(client, root, DEFAULT_EXCLUDES).await? {
        if include.as_ref().is_some_and(|m| !m.is_match(&path)) {
            continue;
        }
        if exclude.as_ref().is_some_and(|m| m.is_match(&path)) {
            continue;
        }
        let file_hits = grep_one(client, &path, &matcher, context_lines).await;
        if file_hits.is_empty() {
            continue;
        }
        files_with_matches.push(path);
        hits.extend(file_hits);
        if hits.len() as i64 >= max_matches {
            break;
        }
    }
    if output_mode == "files" {
        return Ok(json!({ "files": files_with_matches }));
    }
    if output_mode == "count" {
        return Ok(json!({ "count": hits.len(), "files": files_with_matches.len() }));
    }
    let truncated = hits.len() as i64 > max_matches;
    let keep = hits.len().min(max_matches.max(0) as usize);
    hits.truncate(keep);
    Ok(json!({ "matches": hits, "truncated": truncated }))
}

async fn grep_one(
    client: &VolumeClient,
    path: &str,
    matcher: &regex::Regex,
    context_lines: i64,
) -> Vec<Value> {
    // An unreadable file (a directory entry that vanished, a broken blob) is
    // silently skipped, matching the C# `catch (IOException)`.
    let Ok(text) = client.read_text(path).await else {
        return Vec::new();
    };
    let lines = split_lines(&text);
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !matcher.is_match(line) {
            continue;
        }
        let start = (index as i64 - context_lines).max(0);
        let end = (index as i64 + context_lines + 1).min(lines.len() as i64);
        out.push(json!({
            "path": path,
            "line": index + 1,
            "text": line,
            "context": if context_lines != 0 {
                json!(slice(&lines, start, end))
            } else {
                Value::Null
            },
        }));
    }
    out
}

// ─────────────────────────────────────────────────────────────────── tree ─────

/// Recursive JSON tree to `max_depth`, node-capped at 2000.
/// Keys: `path`, `tree`, `truncated`.
pub async fn tree(
    client: &VolumeClient,
    root: &str,
    max_depth: i64,
    exclude_patterns: &[String],
    with_sizes: bool,
) -> Result<Value> {
    let mut excludes: HashSet<String> = TREE_EXCLUDES.iter().map(|s| s.to_string()).collect();
    for e in exclude_patterns {
        excludes.insert(e.clone());
    }
    let counter = AtomicUsize::new(0);
    // `truncated` must mean "entries were left out", not "the counter reached the
    // cap". Deriving it from the counter alone made a tree of exactly TREE_CAP
    // entries report itself as incomplete when nothing had been dropped.
    let hit_cap = AtomicBool::new(false);
    let nodes = build_tree(
        client,
        root.to_string(),
        max_depth,
        &excludes,
        with_sizes,
        &counter,
        &hit_cap,
    )
    .await?;
    Ok(json!({
        "path": root,
        "tree": nodes,
        "truncated": hit_cap.load(Ordering::Relaxed),
    }))
}

/// Boxed because the recursion is async; `&AtomicUsize` keeps the shared node
/// counter `Send` across the recursive futures.
fn build_tree<'a>(
    client: &'a VolumeClient,
    path: String,
    depth: i64,
    excludes: &'a HashSet<String>,
    with_sizes: bool,
    counter: &'a AtomicUsize,
    hit_cap: &'a AtomicBool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send + 'a>> {
    Box::pin(async move {
        if depth < 0 {
            return Ok(Vec::new());
        }
        if counter.load(Ordering::Relaxed) >= TREE_CAP {
            hit_cap.store(true, Ordering::Relaxed);
            return Ok(Vec::new());
        }
        let mut nodes = Vec::new();
        for entry in client.list_dir(&path).await? {
            if excludes.contains(&entry.name) {
                continue;
            }
            // Emit exactly TREE_CAP nodes, then stop and say so. Checking after the
            // increment (as the reference did) dropped the cap-th node, so a tree of
            // exactly TREE_CAP entries came back one short AND flagged as truncated.
            if counter.load(Ordering::Relaxed) >= TREE_CAP {
                hit_cap.store(true, Ordering::Relaxed);
                break;
            }
            counter.fetch_add(1, Ordering::Relaxed);
            let mut node = Map::new();
            node.insert("name".into(), json!(entry.name));
            node.insert("kind".into(), json!(entry.kind));
            if with_sizes && entry.kind == "file" {
                node.insert("size".into(), json!(entry.size));
            }
            if entry.kind == "dir" && depth > 0 {
                let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
                let kids = build_tree(
                    client,
                    child,
                    depth - 1,
                    excludes,
                    with_sizes,
                    counter,
                    hit_cap,
                )
                .await?;
                node.insert("children".into(), Value::Array(kids));
            }
            nodes.push(Value::Object(node));
        }
        Ok(nodes)
    })
}

/// Flat directory listing. Keys: `path`, `entries`, `total`.
pub async fn list_dir(
    client: &VolumeClient,
    path: &str,
    include_hidden: bool,
    sort_by: &str,
    with_sizes: bool,
) -> Result<Value> {
    let raw = client.list_dir(path).await?;
    let mut entries: Vec<(String, i64, Value)> = Vec::new();
    for e in raw {
        if !include_hidden && e.name.starts_with('.') {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("name".into(), json!(e.name));
        entry.insert("kind".into(), json!(e.kind));
        if with_sizes {
            entry.insert("size".into(), json!(e.size));
            entry.insert("mtime".into(), json!(e.mtime));
        }
        // The size key is absent unless with_sizes, so sorting falls back to 0.
        let size = if with_sizes { e.size } else { 0 };
        entries.push((e.name, size, Value::Object(entry)));
    }
    if sort_by == "size" {
        entries.sort_by_key(|t| t.1);
    } else {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let total = entries.len();
    Ok(json!({
        "path": path,
        "entries": entries.into_iter().map(|t| t.2).collect::<Vec<_>>(),
        "total": total,
    }))
}

// ─────────────────────────────────────────────────────────── write / edit ─────

/// Charge the quota, write, audit. The single mutation tail shared by the edit
/// family, so quota accounting can never be skipped.
async fn commit(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    new_text: &str,
    op: &str,
) -> Result<()> {
    let data = new_text.as_bytes();
    safety.charge_write(person, mount_id, data.len() as i64)?;
    client.write_bytes_atomic(norm, data).await?;
    safety.record_audit(person, mount_id, op, norm, "");
    Ok(())
}

/// Create or overwrite a file. No-clobber by default.
/// Keys: `path`, `bytes_written`, `overwritten`, `diff`.
pub async fn write_text(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    content: &str,
    overwrite: bool,
    create_parents: bool,
) -> Result<Value> {
    let exists = client.exists(norm).await?;
    if exists && !overwrite {
        return Err(ToolError::no_clobber(format!("'{norm}' exists (pass overwrite=true)")));
    }
    let mut diff_text = String::new();
    if exists {
        safety.ensure_read_before_write(person, mount_id, norm)?;
        let old = client.read_text(norm).await?;
        diff_text = diff::unified(&old, content, norm);
    }
    if create_parents {
        ensure_parents(client, norm).await?;
    }
    let data = content.as_bytes();
    safety.charge_write(person, mount_id, data.len() as i64)?;
    client.write_bytes_atomic(norm, data).await?;
    // A fresh write counts as a read, so a follow-up edit passes the guard.
    safety.record_read(person, mount_id, norm);
    safety.record_audit(person, mount_id, "write", norm, &format!("{} bytes", data.len()));
    Ok(json!({
        "path": norm,
        "bytes_written": data.len(),
        "overwritten": exists,
        "diff": diff_text,
    }))
}

/// Append to a file, optionally creating it. Keys: `path`, `bytes_appended`.
pub async fn append_text(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    content: &str,
    create: bool,
) -> Result<Value> {
    let exists = client.exists(norm).await?;
    if !exists && !create {
        return Err(ToolError::not_found(format!("'{norm}' does not exist (pass create=true)")));
    }
    let data = content.as_bytes();
    safety.charge_write(person, mount_id, data.len() as i64)?;
    let mut combined = if exists { client.read_bytes(norm).await? } else { Vec::new() };
    combined.extend_from_slice(data);
    client.write_bytes_atomic(norm, &combined).await?;
    safety.record_audit(person, mount_id, "append", norm, &format!("{} bytes", data.len()));
    Ok(json!({ "path": norm, "bytes_appended": data.len() }))
}

/// Touch an empty file. Keys: `path`, `created`.
pub async fn create_empty(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    exist_ok: bool,
) -> Result<Value> {
    if client.exists(norm).await? {
        if !exist_ok {
            return Err(ToolError::no_clobber(format!("'{norm}' already exists")));
        }
        return Ok(json!({ "path": norm, "created": false }));
    }
    client.create_empty(norm).await?;
    safety.record_audit(person, mount_id, "create_empty", norm, "");
    Ok(json!({ "path": norm, "created": true }))
}

/// Replace a unique occurrence (or all of them). `dry_run` returns the diff
/// without writing. Keys: `path`, `applied`, `diff`.
pub async fn edit_unique(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    dry_run: bool,
) -> Result<Value> {
    safety.ensure_read_before_write(person, mount_id, norm)?;
    let old = client.read_text(norm).await?;
    let neu = apply_unique(&old, old_string, new_string, replace_all, norm)?;
    let diff_text = diff::unified(&old, &neu, norm);
    if !dry_run {
        commit(client, safety, person, mount_id, norm, &neu, "edit").await?;
    }
    Ok(json!({ "path": norm, "applied": !dry_run, "diff": diff_text }))
}

/// Apply several edits atomically: every edit must resolve against the text as
/// produced by the previous ones, and nothing is written unless all succeed.
/// Keys: `path`, `applied`, `edits`, `diff`.
pub async fn multi_edit(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    edits: &[Value],
    dry_run: bool,
) -> Result<Value> {
    safety.ensure_read_before_write(person, mount_id, norm)?;
    let old = client.read_text(norm).await?;
    let mut neu = old.clone();
    for spec in edits {
        neu = apply_unique(
            &neu,
            spec.get("old_string").and_then(as_text).unwrap_or_default(),
            spec.get("new_string").and_then(as_text).unwrap_or_default(),
            to_bool(spec.get("replace_all")),
            norm,
        )?;
    }
    let diff_text = diff::unified(&old, &neu, norm);
    if !dry_run {
        commit(client, safety, person, mount_id, norm, &neu, "multi_edit").await?;
    }
    Ok(json!({
        "path": norm,
        "applied": !dry_run,
        "edits": edits.len(),
        "diff": diff_text,
    }))
}

/// Replace a multi-line block, with an optional similarity-based fallback.
/// Keys: `path`, `applied`, `diff`.
pub async fn search_replace(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    search_block: &str,
    replace_block: &str,
    fuzzy: bool,
) -> Result<Value> {
    safety.ensure_read_before_write(person, mount_id, norm)?;
    let old = client.read_text(norm).await?;
    let neu = if old.contains(search_block) {
        replace_first(&old, search_block, replace_block)
    } else if fuzzy {
        fuzzy_replace(&old, search_block, replace_block, norm)?
    } else {
        return Err(ToolError::no_match(format!("search_block not found in '{norm}'")));
    };
    commit(client, safety, person, mount_id, norm, &neu, "search_replace").await?;
    Ok(json!({ "path": norm, "applied": true, "diff": diff::unified(&old, &neu, norm) }))
}

/// Insert content before a 1-based line. Keys: `path`, `applied`, `line`.
pub async fn insert_at_line(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    line: i64,
    content: &str,
) -> Result<Value> {
    safety.ensure_read_before_write(person, mount_id, norm)?;
    let old = client.read_text(norm).await?;
    let lines = diff::split_lines_keep_ends(&old);
    let position = (line - 1).clamp(0, lines.len() as i64) as usize;
    let insert = if content.ends_with('\n') { content.to_string() } else { format!("{content}\n") };
    let neu = format!("{}{insert}{}", lines[..position].concat(), lines[position..].concat());
    commit(client, safety, person, mount_id, norm, &neu, "insert_at_line").await?;
    Ok(json!({ "path": norm, "applied": true, "line": line }))
}

// ──────────────────────────────────────────────────────────────── lifecycle ───

/// Create a directory. Keys: `path`, `created`.
pub async fn mkdir(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    parents: bool,
    exist_ok: bool,
) -> Result<Value> {
    if parents {
        client.makedirs(norm, exist_ok).await?;
    } else if client.exists(norm).await? {
        if !exist_ok {
            return Err(ToolError::no_clobber(format!("'{norm}' already exists")));
        }
    } else {
        client.mkdir(norm).await?;
    }
    safety.record_audit(person, mount_id, "mkdir", norm, "");
    Ok(json!({ "path": norm, "created": true }))
}

/// Delete a path. Soft-deletes into the trash unless `trash=false` AND the
/// server was started with `allow_hard_delete`.
/// Keys: `path`, `trashed`, `trash_path`.
pub async fn delete_path(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    recursive: bool,
    trash: bool,
) -> Result<Value> {
    if !client.exists(norm).await? {
        return Err(ToolError::not_found(format!("'{norm}' does not exist")));
    }
    let is_dir = client.is_dir(norm).await?;
    if is_dir && !recursive {
        return Err(ToolError::invalid_argument(format!(
            "'{norm}' is a directory (pass recursive=true)"
        )));
    }
    let hard = !trash;
    if hard && !safety.config().allow_hard_delete {
        return Err(ToolError::not_supported(
            "hard delete disabled (server started without allow_hard_delete)",
        ));
    }
    let destination = if hard {
        if is_dir {
            client.delete_tree(norm).await?;
        } else {
            client.delete_file(norm).await?;
        }
        None
    } else {
        let dst = safety.trash_path(norm);
        let parent = &dst[..dst.rfind('/').unwrap_or(0)];
        client.makedirs(parent, true).await?;
        client.rename(norm, &dst).await?;
        Some(dst)
    };
    let detail = match &destination {
        Some(d) => format!("-> {d}"),
        None => "hard".to_string(),
    };
    safety.record_audit(person, mount_id, "delete", norm, &detail);
    Ok(json!({
        "path": norm,
        "trashed": !hard,
        "trash_path": match destination { Some(d) => json!(d), None => Value::Null },
    }))
}

/// Rename or relocate. Keys: `source`, `destination`.
pub async fn move_path(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    src: &str,
    dst: &str,
    overwrite: bool,
) -> Result<Value> {
    if !client.exists(src).await? {
        return Err(ToolError::not_found(format!("'{src}' does not exist")));
    }
    if client.exists(dst).await? {
        if !overwrite {
            return Err(ToolError::no_clobber(format!("'{dst}' exists (pass overwrite=true)")));
        }
        // The metadata store refuses to rename onto an existing path, so the flag was
        // accepted and then ignored: `overwrite: true` always failed with NO_CLOBBER.
        // Clear the destination first, GCing whatever it referenced.
        if client.is_dir(dst).await? {
            client.delete_tree(dst).await?;
        } else {
            client.delete_file(dst).await?;
        }
    }
    client.rename(src, dst).await?;
    safety.record_audit(person, mount_id, "move", src, &format!("-> {dst}"));
    Ok(json!({ "source": src, "destination": dst }))
}

/// Copy a file or, with `recursive`, a whole tree. Keys: `source`, `destination`.
/// Only the single-file path charges the write quota, as in the C# original.
pub async fn copy_path(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    src: &str,
    dst: &str,
    overwrite: bool,
    recursive: bool,
) -> Result<Value> {
    if !client.exists(src).await? {
        return Err(ToolError::not_found(format!("'{src}' does not exist")));
    }
    if client.exists(dst).await? && !overwrite {
        return Err(ToolError::no_clobber(format!("'{dst}' exists (pass overwrite=true)")));
    }
    if client.is_dir(src).await? {
        if !recursive {
            return Err(ToolError::invalid_argument(format!(
                "'{src}' is a directory (pass recursive=true)"
            )));
        }
        client.copy_tree(src, dst).await?;
    } else {
        let data = client.read_bytes(src).await?;
        safety.charge_write(person, mount_id, data.len() as i64)?;
        // No explicit makedirs: the metadata store creates missing parents on
        // put_file, exactly like the C# path.
        client.write_bytes_atomic(dst, &data).await?;
    }
    safety.record_audit(person, mount_id, "copy", src, &format!("-> {dst}"));
    Ok(json!({ "source": src, "destination": dst }))
}

// ──────────────────────────────────────────────────────────────── helpers ─────

async fn ensure_parents(client: &VolumeClient, norm: &str) -> Result<()> {
    let parent = match norm.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => norm[..i].to_string(),
    };
    if parent != "/" {
        client.makedirs(&parent, true).await?;
    }
    Ok(())
}

/// Clamped half-open slice, tolerant of out-of-range and negative bounds.
fn slice(lines: &[String], start: i64, end: i64) -> &[String] {
    let len = lines.len() as i64;
    let s = start.clamp(0, len);
    let e = end.clamp(0, len);
    if s >= e {
        return &[];
    }
    &lines[s as usize..e as usize]
}

/// `"{n}\t{line}"` per line, joined with '\n'. `start` is the 1-based number of
/// the first line.
fn number_lines(lines: &[String], start: i64) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&(start + i as i64).to_string());
        out.push('\t');
        out.push_str(line);
    }
    out
}

/// Leading-whitespace width, the C# `IndentOf`.
fn indent_of(line: &str) -> usize {
    line.chars().count() - line.trim_start().chars().count()
}

/// Grow an indentation block around `anchor`: upwards to the first less-indented
/// non-blank line, downwards until indentation drops below the anchor's.
fn indent_block(lines: &[String], anchor: i64, max_lines: i64) -> Result<(usize, usize)> {
    if lines.is_empty() {
        return Err(ToolError::invalid_argument("file is empty"));
    }
    let anchor = anchor.clamp(0, lines.len() as i64 - 1) as usize;
    let base_indent = indent_of(&lines[anchor]);
    let mut start = anchor;
    while start > 0 {
        let previous = &lines[start - 1];
        // Include the owning header line, then stop.
        if !previous.trim().is_empty() && indent_of(previous) < base_indent {
            start -= 1;
            break;
        }
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < lines.len() && ((end - start) as i64) < max_lines {
        if !lines[end].trim().is_empty() && indent_of(&lines[end]) < base_indent {
            break;
        }
        end += 1;
    }
    Ok((start, end))
}

fn count_occurrences(text: &str, sub: &str) -> usize {
    if sub.is_empty() {
        return 0;
    }
    text.matches(sub).count()
}

fn replace_first(text: &str, search: &str, replace: &str) -> String {
    match text.find(search) {
        None => text.to_string(),
        Some(i) => format!("{}{replace}{}", &text[..i], &text[i + search.len()..]),
    }
}

/// `ERR_NO_MATCH` when absent, `ERR_AMBIGUOUS_MATCH` when several sites match and
/// `replace_all` was not requested.
fn apply_unique(
    text: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    path: &str,
) -> Result<String> {
    let count = count_occurrences(text, old_string);
    if count == 0 {
        return Err(ToolError::no_match(format!("old_string not found in '{path}'")));
    }
    if count > 1 && !replace_all {
        return Err(ToolError::ambiguous_match(format!(
            "old_string matches {count} sites in '{path}' (use replace_all)"
        )));
    }
    Ok(if replace_all {
        text.replace(old_string, new_string)
    } else {
        replace_first(text, old_string, new_string)
    })
}

/// Best-effort block replace: slide a window of the search block's line count
/// over the file and keep the most similar candidate above the threshold.
fn fuzzy_replace(
    text: &str,
    search_block: &str,
    replace_block: &str,
    path: &str,
) -> Result<String> {
    let lines = diff::split_lines_keep_ends(text);
    let span = diff::split_lines_keep_ends(search_block).len();
    let mut best_ratio = 0.0f64;
    let mut best_index: i64 = -1;
    let bound = (lines.len() as i64 - span as i64 + 1).max(0);
    for start in 0..bound {
        let s = start as usize;
        let candidate = lines[s..(s + span).min(lines.len())].concat();
        let ratio = similarity_ratio(&candidate, search_block);
        if ratio > best_ratio {
            best_ratio = ratio;
            best_index = start;
        }
    }
    if best_index < 0 || best_ratio < FUZZY_THRESHOLD {
        return Err(ToolError::no_match(format!("no fuzzy match for search_block in '{path}'")));
    }
    let block = if replace_block.ends_with('\n') {
        replace_block.to_string()
    } else {
        format!("{replace_block}\n")
    };
    let bi = best_index as usize;
    let tail_start = (bi + span).min(lines.len());
    Ok(format!(
        "{}{block}{}",
        lines[..bi].concat(),
        lines[tail_start..].concat()
    ))
}

/// LCS-based similarity in `0.0..=1.0`, the C# `Similarity.Ratio` (an
/// approximation of Python's `difflib.SequenceMatcher.ratio`).
fn similarity_ratio(a: &str, b: &str) -> f64 {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let total = av.len() + bv.len();
    if total == 0 {
        return 1.0;
    }
    let mut prev = vec![0usize; bv.len() + 1];
    let mut cur = vec![0usize; bv.len() + 1];
    for i in 1..=av.len() {
        for j in 1..=bv.len() {
            cur[j] = if av[i - 1] == bv[j - 1] {
                prev[j - 1] + 1
            } else {
                prev[j].max(cur[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.iter_mut().for_each(|v| *v = 0);
    }
    2.0 * prev[bv.len()] as f64 / total as f64
}

fn as_text(v: &Value) -> Option<&str> {
    v.as_str()
}

/// The C# `ToBool`: a real bool, or the literal string "true" (any case).
fn to_bool(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn kind_of(mode: i64) -> &'static str {
    match mode & 0xF000 {
        0x4000 => "dir",
        0xA000 => "symlink",
        0x8000 => "file",
        _ => "other",
    }
}

fn oct_permissions(mode: i64) -> String {
    format!("0o{:o}", mode & 0xFFF)
}

/// Extension-driven MIME lookup, the same subset as the C# `Mime.Guess`.
fn mime_guess(path: &str) -> Option<&'static str> {
    let i = path.rfind('.')?;
    let ext = path[i..].to_ascii_lowercase();
    let m = match ext.as_str() {
        ".txt" | ".text" | ".log" => "text/plain",
        ".md" | ".markdown" => "text/markdown",
        ".html" | ".htm" => "text/html",
        ".css" => "text/css",
        ".csv" => "text/csv",
        ".json" => "application/json",
        ".xml" => "application/xml",
        ".js" | ".mjs" => "text/javascript",
        ".yaml" | ".yml" => "application/yaml",
        ".py" => "text/x-python",
        ".c" => "text/x-csrc",
        ".h" => "text/x-chdr",
        ".pdf" => "application/pdf",
        ".png" => "image/png",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".gif" => "image/gif",
        ".bmp" => "image/bmp",
        ".webp" => "image/webp",
        ".tif" | ".tiff" => "image/tiff",
        ".zip" => "application/zip",
        ".gz" => "application/gzip",
        ".docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ".pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => return None,
    };
    Some(m)
}

/// md5 and sha1 come from the audited RustCrypto crates rather than a local
/// implementation: `fs.hash` advertises them as content hashes, and hand rolling a
/// digest is avoidable risk. sha256 comes from the same family via `VolumeClient`.
mod hashing {
    use md5::Digest as _;

    pub fn md5_hex(data: &[u8]) -> String {
        let mut h = md5::Md5::new();
        h.update(data);
        hex::encode(h.finalize())
    }

    pub fn sha1_hex(data: &[u8]) -> String {
        let mut h = sha1::Sha1::new();
        h.update(data);
        hex::encode(h.finalize())
    }


    #[cfg(test)]
    mod tests {
        use super::*;

        /// Known answer vectors, so a crate swap cannot silently change `fs.hash`.
        #[test]
        fn md5_known_answers() {
            assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
            assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        }

        #[test]
        fn sha1_known_answers() {
            assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
            assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        }

        /// Multi block input, past the 64 byte compression boundary.
        #[test]
        fn multi_block_input() {
            let data = vec![b'a'; 1000];
            assert_eq!(md5_hex(&data), "cabe45dcc9ae5b66ba86600cca6b8ba8");
            assert_eq!(sha1_hex(&data), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
        }
    }
}

// ─────────────────────────────────────── symbol search and documents ────
//
// These four compositions sit here, in the engine, rather than in a tool module,
// because BOTH the MCP tool layer and the REST data plane need them. Keeping two
// copies (which is how they first landed) is a drift hazard: a fix on one path
// would silently leave the other on the old behaviour. Walking, symbol lookup,
// extraction and docx rendering themselves live in `crate::docs`.

/// Symbol definitions across a subtree. Key: `definitions`, entries
/// `{path, name, kind, line}`.
pub async fn find_definitions(
    client: &VolumeClient,
    root: &str,
    name: &str,
    kind: Option<&str>,
) -> Result<Value> {
    let mut out: Vec<Value> = Vec::new();
    for (path, _) in iter_files(client, root, DEFAULT_EXCLUDES).await? {
        // No grammar and no lexical pattern for this extension: skip the read
        // entirely, like the reference does.
        if crate::docs::language_for(&path).is_none() {
            continue;
        }
        let source = client.read_text(&path).await?;
        for d in crate::docs::find_definitions(&path, &source, name, kind) {
            out.push(json!({"path": d.path, "name": d.name, "kind": d.kind, "line": d.line}));
        }
    }
    Ok(json!({"definitions": out}))
}

/// Identifier references across a subtree. Key: `references`, entries
/// `{path, line, kind}`. The name is the query, so it is not echoed per hit.
pub async fn find_references(client: &VolumeClient, root: &str, name: &str) -> Result<Value> {
    if name.is_empty() {
        return Err(ToolError::invalid_argument("name is required"));
    }
    let mut out: Vec<Value> = Vec::new();
    for (path, _) in iter_files(client, root, DEFAULT_EXCLUDES).await? {
        if crate::docs::language_for(&path).is_none() {
            continue;
        }
        let source = client.read_text(&path).await?;
        for m in crate::docs::find_references(&path, &source, name) {
            out.push(json!({"path": m.path, "line": m.line, "kind": m.kind}));
        }
    }
    Ok(json!({"references": out}))
}

/// Extract a document to a companion `.md` and account for the write.
///
/// A cache hit wrote nothing, so it is not charged, not audited, and does not
/// record a read. A miss charges the companion's size, records the read (so a
/// follow-up edit on the companion passes the guard) and audits it.
#[allow(clippy::too_many_arguments)]
pub async fn extract_document(
    client: &VolumeClient,
    safety: &SafetyManager,
    ocr_config: &crate::config::OcrConfig,
    person: &str,
    mount_id: &str,
    norm: &str,
    max_chars: usize,
    preview_chars: usize,
    ocr: bool,
    refresh: bool,
) -> Result<Value> {
    let provider = crate::docs::provider_from_config(ocr_config);
    let payload = crate::docs::extract_text(
        client,
        provider.as_ref(),
        norm,
        max_chars,
        preview_chars,
        ocr,
        refresh,
    )
    .await?;

    let cached = payload.get("cached").and_then(Value::as_bool).unwrap_or(false);
    if let Some(md) = payload.get("md_path").and_then(Value::as_str)
        && !cached
    {
        let bytes = client.stat(md).await?.size;
        safety.charge_write(person, mount_id, bytes)?;
        safety.record_read(person, mount_id, md);
        safety.record_audit(person, mount_id, "extract_text", md, &format!("{bytes} bytes"));
    }
    Ok(payload)
}

/// Render Markdown to a `.docx` and store it. Keys: `path`, `bytes_written`,
/// `overwritten`.
#[allow(clippy::too_many_arguments)]
pub async fn write_docx(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    norm: &str,
    markdown: &str,
    title: Option<&str>,
    overwrite: bool,
) -> Result<Value> {
    if !norm.to_ascii_lowercase().ends_with(".docx") {
        return Err(ToolError::invalid_argument("path must end with .docx"));
    }
    let exists = client.exists(norm).await?;
    if exists && !overwrite {
        return Err(ToolError::no_clobber(format!("'{norm}' exists (pass overwrite=true)")));
    }
    if exists {
        safety.ensure_read_before_write(person, mount_id, norm)?;
    }
    let data = crate::docs::render_markdown_to_docx(markdown, title)?;
    let parent = match norm.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => norm[..i].to_string(),
    };
    if parent != "/" {
        client.makedirs(&parent, true).await?;
    }
    safety.charge_write(person, mount_id, data.len() as i64)?;
    client.write_bytes_atomic(norm, &data).await?;
    safety.record_read(person, mount_id, norm);
    safety.record_audit(person, mount_id, "write_docx", norm, &format!("{} bytes", data.len()));
    Ok(json!({"path": norm, "bytes_written": data.len(), "overwritten": exists}))
}

// ────────────────────────────────────────────────── apply_patch (V4A engine) ────

/// Apply every operation of a V4A patch, in order. Key: `files`, one entry per
/// touched path.
///
/// The parser and applier live here, in the engine, rather than in the tool module
/// where they first landed: the REST plane needs the same operation, and it had to
/// reach back through the tool registry to get it. One implementation, two thin
/// adapters, like every other operation.
pub async fn apply_patch(
    client: &VolumeClient,
    safety: &SafetyManager,
    person: &str,
    mount: &str,
    patch_text: &str,
) -> Result<Value> {
    let ops = parse_patch(patch_text)?;
    let mut touched: Vec<Value> = Vec::with_capacity(ops.len());
    for op in &ops {
        let norm = safety.normalize_path(&op.path)?;
        match op.kind {
            OpKind::Add => {
                let data = op.add_content.as_bytes();
                safety.charge_write(person, mount, data.len() as i64)?;
                client.write_bytes_atomic(&norm, data).await?;
                touched.push(json!({"path": norm, "op": "add"}));
            }
            OpKind::Delete => {
                safety.ensure_read_before_write(person, mount, &norm)?;
                client.delete_file(&norm).await?;
                touched.push(json!({"path": norm, "op": "delete"}));
            }
            OpKind::Update => {
                safety.ensure_read_before_write(person, mount, &norm)?;
                let old = client.read_text(&norm).await?;
                let neu = apply_update(&old, op)?;
                let data = neu.as_bytes();
                safety.charge_write(person, mount, data.len() as i64)?;
                client.write_bytes_atomic(&norm, data).await?;
                // Two audit entries per updated file, because the C# writes one
                // inside its shared Commit helper and one in the loop tail. The
                // duplicate is observable through fs.audit_log, so it is kept.
                safety.record_audit(person, mount, "apply_patch", &norm, "");
                match &op.move_to {
                    Some(target) => {
                        let dst = safety.normalize_path(target)?;
                        client.rename(&norm, &dst).await?;
                        touched.push(json!({"path": norm, "op": "update", "moved_to": dst}));
                    }
                    None => touched.push(json!({"path": norm, "op": "update"})),
                }
            }
        }
        safety.record_audit(person, mount, "apply_patch", &norm, "");
    }
    Ok(json!({"files": touched}))
}

const BEGIN: &str = "*** Begin Patch";
const END: &str = "*** End Patch";
const ADD: &str = "*** Add File: ";
const UPDATE: &str = "*** Update File: ";
const DELETE: &str = "*** Delete File: ";
const MOVE: &str = "*** Move to: ";
const HUNK_MARKER: &str = "@@";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpKind {
    Add,
    Update,
    Delete,
}

/// One hunk: the removed and added lines, plus the context that frames them.
#[derive(Debug, Default)]
struct Hunk {
    removed: Vec<String>,
    added: Vec<String>,
    context_before: Vec<String>,
    context_after: Vec<String>,
}

#[derive(Debug)]
struct FileOp {
    kind: OpKind,
    path: String,
    move_to: Option<String>,
    add_content: String,
    hunks: Vec<Hunk>,
}

impl FileOp {
    fn new(kind: OpKind, path: String) -> Self {
        Self { kind, path, move_to: None, add_content: String::new(), hunks: Vec::new() }
    }
}

/// Parse the `*** Begin Patch` envelope into file operations.
fn parse_patch(text: &str) -> Result<Vec<FileOp>> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    if lines.first().map(|l| l.trim()) != Some(BEGIN) {
        return Err(ToolError::invalid_argument("patch must start with '*** Begin Patch'"));
    }
    let mut ops: Vec<FileOp> = Vec::new();
    let mut index = 1;
    while index < lines.len() {
        let line = lines[index];
        if line.trim() == END {
            return Ok(ops);
        }
        if let Some(rest) = line.strip_prefix(ADD) {
            index = parse_add(&lines, index + 1, rest.trim().to_string(), &mut ops);
        } else if let Some(rest) = line.strip_prefix(UPDATE) {
            index = parse_update(&lines, index + 1, rest.trim().to_string(), &mut ops);
        } else if let Some(rest) = line.strip_prefix(DELETE) {
            ops.push(FileOp::new(OpKind::Delete, rest.trim().to_string()));
            index += 1;
        } else {
            return Err(ToolError::invalid_argument(format!("unexpected patch line: '{line}'")));
        }
    }
    Err(ToolError::invalid_argument("patch missing '*** End Patch'"))
}

/// Body of an `Add File` block: every line, with one leading '+' stripped.
fn parse_add(lines: &[&str], mut index: usize, path: String, ops: &mut Vec<FileOp>) -> usize {
    let mut body: Vec<&str> = Vec::new();
    while index < lines.len() && !is_marker(lines[index]) {
        body.push(lines[index].strip_prefix('+').unwrap_or(lines[index]));
        index += 1;
    }
    let mut op = FileOp::new(OpKind::Add, path);
    op.add_content = body.join("\n");
    ops.push(op);
    index
}

/// Body of an `Update File` block: an optional `Move to:` then hunks.
fn parse_update(lines: &[&str], mut index: usize, path: String, ops: &mut Vec<FileOp>) -> usize {
    let mut op = FileOp::new(OpKind::Update, path);
    if let Some(target) = lines.get(index).and_then(|l| l.strip_prefix(MOVE)) {
        op.move_to = Some(target.trim().to_string());
        index += 1;
    }
    // `current` is an index into op.hunks so the borrow checker stays out of the
    // way while the vector grows.
    let mut current: Option<usize> = None;
    while index < lines.len() && !is_file_marker(lines[index]) {
        let raw = lines[index];
        if raw.starts_with(HUNK_MARKER) {
            op.hunks.push(Hunk::default());
            current = Some(op.hunks.len() - 1);
            index += 1;
            continue;
        }
        let slot = match current {
            Some(i) => i,
            None => {
                op.hunks.push(Hunk::default());
                current = Some(op.hunks.len() - 1);
                op.hunks.len() - 1
            }
        };
        classify_line(raw, &mut op.hunks[slot]);
        index += 1;
    }
    ops.push(op);
    index
}

/// '+' adds, '-' removes, anything else is context: before the first change when
/// the hunk is still empty, after it otherwise.
fn classify_line(raw: &str, hunk: &mut Hunk) {
    if let Some(rest) = raw.strip_prefix('+') {
        hunk.added.push(rest.to_string());
    } else if let Some(rest) = raw.strip_prefix('-') {
        hunk.removed.push(rest.to_string());
    } else {
        let text = raw.strip_prefix(' ').unwrap_or(raw).to_string();
        if !hunk.removed.is_empty() || !hunk.added.is_empty() {
            hunk.context_after.push(text);
        } else {
            hunk.context_before.push(text);
        }
    }
}

fn is_marker(line: &str) -> bool {
    line.trim() == END || is_file_marker(line)
}

fn is_file_marker(line: &str) -> bool {
    line.starts_with(ADD) || line.starts_with(UPDATE) || line.starts_with(DELETE) || line.trim() == END
}

/// Rebuild the old block (context + removed) and swap in the new one. An empty
/// old block means "replace the whole file with the new block".
fn apply_update(original: &str, op: &FileOp) -> Result<String> {
    let mut content = original.to_string();
    for hunk in &op.hunks {
        let old_block = join_block(&hunk.context_before, &hunk.removed, &hunk.context_after);
        let new_block = join_block(&hunk.context_before, &hunk.added, &hunk.context_after);
        if !old_block.is_empty() && !content.contains(&old_block) {
            return Err(ToolError::no_match(format!(
                "hunk context not found in '{}'",
                op.path
            )));
        }
        content = if old_block.is_empty() {
            new_block
        } else {
            replace_first(&content, &old_block, &new_block)
        };
    }
    Ok(content)
}

fn join_block(before: &[String], middle: &[String], after: &[String]) -> String {
    let mut all: Vec<&str> = Vec::with_capacity(before.len() + middle.len() + after.len());
    all.extend(before.iter().map(String::as_str));
    all.extend(middle.iter().map(String::as_str));
    all.extend(after.iter().map(String::as_str));
    all.join("\n")
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SafetyConfig;
    use crate::errors::code;
    use std::sync::Arc;

    const P: &str = "a@b.c";
    const M: &str = "proj";

    struct Fix {
        _dir: tempfile::TempDir,
        v: VolumeClient,
        s: SafetyManager,
    }

    fn fixture() -> Fix {
        fixture_with(SafetyConfig::default())
    }

    fn fixture_with(cfg: SafetyConfig) -> Fix {
        let meta = Arc::new(crate::storage::meta::SqliteMetaStore::in_memory().unwrap());
        let d = tempfile::tempdir().unwrap();
        let blob = Arc::new(crate::storage::blob::local::LocalBlobStore::new(d.path(), "b"));
        Fix {
            _dir: d,
            v: VolumeClient::new("p", meta, blob),
            s: SafetyManager::new(cfg),
        }
    }

    /// Seed a file and mark it read, the usual precondition for the edit family.
    async fn seed(f: &Fix, path: &str, text: &str) {
        f.v.write_text_atomic(path, text).await.unwrap();
        f.s.record_read(P, M, path);
    }

    fn s(v: &Value, key: &str) -> String {
        v[key].as_str().unwrap().to_string()
    }

    // ── read ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn read_window_numbers_lines_and_reports_total() {
        let f = fixture();
        seed(&f, "/a.txt", "one\ntwo\nthree\n").await;
        let r = read_window(&f.v, &f.s, P, M, "/a.txt", 0, 2000, true).await.unwrap();
        assert_eq!(s(&r, "content"), "1\tone\n2\ttwo\n3\tthree");
        assert_eq!(r["total_lines"], 3);
        assert_eq!(r["truncated"], false);
        assert_eq!(r["next_offset"], Value::Null);
    }

    #[tokio::test]
    async fn read_window_without_numbers_is_raw_join() {
        let f = fixture();
        seed(&f, "/a.txt", "one\ntwo\n").await;
        let r = read_window(&f.v, &f.s, P, M, "/a.txt", 0, 2000, false).await.unwrap();
        assert_eq!(s(&r, "content"), "one\ntwo");
    }

    #[tokio::test]
    async fn read_window_pages_and_sets_next_offset() {
        let f = fixture();
        let body: String = (1..=10).map(|i| format!("l{i}\n")).collect();
        seed(&f, "/a.txt", &body).await;
        let r = read_window(&f.v, &f.s, P, M, "/a.txt", 0, 4, true).await.unwrap();
        assert_eq!(s(&r, "content"), "1\tl1\n2\tl2\n3\tl3\n4\tl4");
        assert_eq!(r["truncated"], true);
        assert_eq!(r["next_offset"], 4);

        let r2 = read_window(&f.v, &f.s, P, M, "/a.txt", 8, 4, true).await.unwrap();
        assert_eq!(s(&r2, "content"), "9\tl9\n10\tl10");
        assert_eq!(r2["truncated"], false);
    }

    /// The per-call limit is bounded by safety.max_read_lines.
    #[tokio::test]
    async fn read_window_respects_max_read_lines_cap() {
        let f = fixture_with(SafetyConfig { max_read_lines: 2, ..Default::default() });
        seed(&f, "/a.txt", "a\nb\nc\nd\n").await;
        let r = read_window(&f.v, &f.s, P, M, "/a.txt", 0, 1000, true).await.unwrap();
        assert_eq!(s(&r, "content"), "1\ta\n2\tb");
        assert_eq!(r["next_offset"], 2);
    }

    #[tokio::test]
    async fn read_window_on_empty_file() {
        let f = fixture();
        seed(&f, "/e.txt", "").await;
        let r = read_window(&f.v, &f.s, P, M, "/e.txt", 0, 2000, true).await.unwrap();
        assert_eq!(s(&r, "content"), "");
        assert_eq!(r["total_lines"], 0);
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn read_window_missing_file_is_not_found() {
        let f = fixture();
        let e = read_window(&f.v, &f.s, P, M, "/nope.txt", 0, 10, true).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    /// A read is what unlocks a later edit.
    #[tokio::test]
    async fn read_records_the_read_guard() {
        let f = fixture();
        f.v.write_text_atomic("/g.txt", "x\n").await.unwrap();
        let e = edit_unique(&f.v, &f.s, P, M, "/g.txt", "x", "y", false, false).await.unwrap_err();
        assert_eq!(e.code, code::EDIT_WITHOUT_PRIOR_READ);

        read_window(&f.v, &f.s, P, M, "/g.txt", 0, 10, true).await.unwrap();
        let r = edit_unique(&f.v, &f.s, P, M, "/g.txt", "x", "y", false, false).await.unwrap();
        assert_eq!(r["applied"], true);
    }

    #[tokio::test]
    async fn read_bytes_is_base64_with_mime() {
        let f = fixture();
        seed(&f, "/a.json", "{\"k\":1}").await;
        let r = read_bytes_b64(&f.v, &f.s, P, M, "/a.json", 0, 65536).await.unwrap();
        assert_eq!(s(&r, "base64"), "eyJrIjoxfQ==");
        assert_eq!(s(&r, "mime_type"), "application/json");
        assert_eq!(r["length"], 7);
    }

    #[tokio::test]
    async fn read_bytes_slices_and_defaults_mime() {
        let f = fixture();
        seed(&f, "/blob.bin", "0123456789").await;
        let r = read_bytes_b64(&f.v, &f.s, P, M, "/blob.bin", 4, 3).await.unwrap();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(s(&r, "base64"))
            .unwrap();
        assert_eq!(raw, b"456");
        assert_eq!(s(&r, "mime_type"), "application/octet-stream");
        assert_eq!(r["length"], 3);
    }

    #[tokio::test]
    async fn read_lines_is_inclusive_and_one_based() {
        let f = fixture();
        seed(&f, "/a.txt", "a\nb\nc\nd\n").await;
        let r = read_lines(&f.v, &f.s, P, M, "/a.txt", 2, 3).await.unwrap();
        assert_eq!(s(&r, "content"), "2\tb\n3\tc");
        assert_eq!(r["total_lines"], 4);
    }

    #[tokio::test]
    async fn read_lines_clamps_out_of_range_bounds() {
        let f = fixture();
        seed(&f, "/a.txt", "a\nb\n").await;
        let r = read_lines(&f.v, &f.s, P, M, "/a.txt", 0, 99).await.unwrap();
        assert_eq!(s(&r, "content"), "1\ta\n2\tb");
        let empty = read_lines(&f.v, &f.s, P, M, "/a.txt", 5, 9).await.unwrap();
        assert_eq!(s(&empty, "content"), "");
    }

    #[tokio::test]
    async fn read_section_grows_the_indentation_block() {
        let f = fixture();
        let src = "def outer():\n    a = 1\n    b = 2\nnext_top = 3\n";
        seed(&f, "/x.py", src).await;
        // anchor on "b = 2" pulls in the def line above and stops at the dedent
        let r = read_section(&f.v, &f.s, P, M, "/x.py", 3, 200).await.unwrap();
        assert_eq!(r["start_line"], 1);
        assert_eq!(r["end_line"], 3);
        assert_eq!(s(&r, "content"), "1\tdef outer():\n2\t    a = 1\n3\t    b = 2");
    }

    #[tokio::test]
    async fn read_section_honours_max_lines() {
        let f = fixture();
        let src = "top\n  a\n  b\n  c\n  d\n";
        seed(&f, "/x.txt", src).await;
        let r = read_section(&f.v, &f.s, P, M, "/x.txt", 2, 2).await.unwrap();
        assert_eq!(r["start_line"], 1);
        assert_eq!(r["end_line"], 2);
    }

    #[tokio::test]
    async fn read_section_on_empty_file_is_invalid_argument() {
        let f = fixture();
        seed(&f, "/e.txt", "").await;
        let e = read_section(&f.v, &f.s, P, M, "/e.txt", 1, 10).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("file is empty"));
    }

    #[tokio::test]
    async fn read_many_isolates_per_file_errors() {
        let f = fixture();
        seed(&f, "/ok.txt", "hello\n").await;
        let paths = vec!["/ok.txt".to_string(), "/missing.txt".to_string()];
        let r = read_many(&f.v, &f.s, P, M, &paths, 500).await.unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(s(&files[0], "path"), "/ok.txt");
        assert_eq!(s(&files[0], "content"), "1\thello");
        assert_eq!(files[0]["truncated"], false);
        assert_eq!(s(&files[1], "path"), "/missing.txt");
        assert_eq!(s(&files[1], "error"), "not found: /missing.txt");
        assert!(files[1].get("content").is_none());
    }

    #[tokio::test]
    async fn read_many_normalizes_and_truncates() {
        let f = fixture();
        seed(&f, "/d/a.txt", "1\n2\n3\n").await;
        let paths = vec!["d/../d/a.txt".to_string()];
        let r = read_many(&f.v, &f.s, P, M, &paths, 2).await.unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(s(&files[0], "path"), "/d/a.txt", "path is normalized");
        assert_eq!(s(&files[0], "content"), "1\t1\n2\t2");
        assert_eq!(files[0]["truncated"], true);
    }

    #[tokio::test]
    async fn head_and_tail_number_real_line_positions() {
        let f = fixture();
        let body: String = (1..=6).map(|i| format!("l{i}\n")).collect();
        seed(&f, "/a.txt", &body).await;
        let h = head(&f.v, &f.s, P, M, "/a.txt", 2).await.unwrap();
        assert_eq!(s(&h, "content"), "1\tl1\n2\tl2");
        let t = tail(&f.v, &f.s, P, M, "/a.txt", 2).await.unwrap();
        assert_eq!(s(&t, "content"), "5\tl5\n6\tl6");
    }

    #[tokio::test]
    async fn tail_more_lines_than_the_file_has() {
        let f = fixture();
        seed(&f, "/a.txt", "only\n").await;
        let t = tail(&f.v, &f.s, P, M, "/a.txt", 50).await.unwrap();
        assert_eq!(s(&t, "content"), "1\tonly");
    }

    #[tokio::test]
    async fn count_lines_ignores_the_trailing_terminator() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "a\nb\n").await.unwrap();
        assert_eq!(count_lines(&f.v, "/a.txt").await.unwrap()["total_lines"], 2);
        f.v.write_text_atomic("/b.txt", "a\nb").await.unwrap();
        assert_eq!(count_lines(&f.v, "/b.txt").await.unwrap()["total_lines"], 2);
        f.v.write_text_atomic("/c.txt", "").await.unwrap();
        assert_eq!(count_lines(&f.v, "/c.txt").await.unwrap()["total_lines"], 0);
    }

    #[tokio::test]
    async fn unicode_content_round_trips_through_read() {
        let f = fixture();
        seed(&f, "/u.txt", "héllo ✅\nmünd\n").await;
        let r = read_window(&f.v, &f.s, P, M, "/u.txt", 0, 10, true).await.unwrap();
        assert_eq!(s(&r, "content"), "1\théllo ✅\n2\tmünd");
        assert_eq!(r["total_lines"], 2);
    }

    // ── metadata ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn stat_reports_posix_shape() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "12345").await.unwrap();
        let r = stat_info(&f.v, "/a.txt").await.unwrap();
        assert_eq!(s(&r, "path"), "/a.txt");
        assert_eq!(r["size"], 5);
        assert_eq!(s(&r, "mode"), "0o644");
        assert_eq!(s(&r, "kind"), "file");
        assert_eq!(r["uid"], 1000);
        assert_eq!(r["gid"], 1000);
        assert!(r["mtime"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn stat_on_a_directory() {
        let f = fixture();
        f.v.mkdir("/d").await.unwrap();
        let r = stat_info(&f.v, "/d").await.unwrap();
        assert_eq!(s(&r, "kind"), "dir");
        assert_eq!(s(&r, "mode"), "0o755");
    }

    #[tokio::test]
    async fn stat_missing_is_not_found() {
        let f = fixture();
        let e = stat_info(&f.v, "/nope").await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn exists_reports_kind_or_null() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        let r = exists_info(&f.v, "/a.txt").await.unwrap();
        assert_eq!(r["exists"], true);
        assert_eq!(s(&r, "kind"), "file");

        let r2 = exists_info(&f.v, "/nope").await.unwrap();
        assert_eq!(r2["exists"], false);
        assert_eq!(r2["kind"], Value::Null);
    }

    #[tokio::test]
    async fn hash_supports_the_four_algorithms() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "abc").await.unwrap();
        let md5 = hash_file(&f.v, "/a.txt", "md5").await.unwrap();
        assert_eq!(s(&md5, "hash"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(s(&md5, "algo"), "md5");
        assert_eq!(md5["size"], 3);

        let sha1 = hash_file(&f.v, "/a.txt", "sha1").await.unwrap();
        assert_eq!(s(&sha1, "hash"), "a9993e364706816aba3e25717850c26c9cd0d89d");

        let sha256 = hash_file(&f.v, "/a.txt", "sha256").await.unwrap();
        assert_eq!(
            s(&sha256, "hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let sha512 = hash_file(&f.v, "/a.txt", "sha512").await.unwrap();
        assert!(s(&sha512, "hash").starts_with("ddaf35a193617aba"));
        assert_eq!(s(&sha512, "hash").len(), 128);
    }

    #[tokio::test]
    async fn hash_of_an_empty_file_matches_known_digests() {
        let f = fixture();
        f.v.create_empty("/e.txt").await.unwrap();
        assert_eq!(
            s(&hash_file(&f.v, "/e.txt", "md5").await.unwrap(), "hash"),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            s(&hash_file(&f.v, "/e.txt", "sha1").await.unwrap(), "hash"),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
    }

    #[tokio::test]
    async fn hash_rejects_an_unknown_algorithm() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        let e = hash_file(&f.v, "/a.txt", "crc32").await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("unsupported algo 'crc32'"));
    }

    /// A multi-block payload exercises the md5/sha1 chunk loops.
    #[tokio::test]
    async fn hash_of_a_multi_block_payload() {
        let f = fixture();
        let body = "a".repeat(1000);
        f.v.write_text_atomic("/big.txt", &body).await.unwrap();
        assert_eq!(
            s(&hash_file(&f.v, "/big.txt", "md5").await.unwrap(), "hash"),
            "cabe45dcc9ae5b66ba86600cca6b8ba8"
        );
        assert_eq!(
            s(&hash_file(&f.v, "/big.txt", "sha1").await.unwrap(), "hash"),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    // ── search ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn glob_matches_by_path_or_name_newest_first() {
        let f = fixture();
        f.v.write_text_atomic("/src/a.rs", "1").await.unwrap();
        f.v.write_text_atomic("/src/b.txt", "2").await.unwrap();
        f.v.write_text_atomic("/c.rs", "3").await.unwrap();
        let r = glob_files(&f.v, "/", "*.rs", &[]).await.unwrap();
        let matches: Vec<&str> = r["matches"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(matches.len(), 2, "got {matches:?}");
        assert!(matches.contains(&"/src/a.rs"));
        assert!(matches.contains(&"/c.rs"));
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn glob_prunes_default_excludes_and_extra_excludes() {
        let f = fixture();
        f.v.write_text_atomic("/keep.rs", "1").await.unwrap();
        f.v.write_text_atomic("/target/gen.rs", "2").await.unwrap();
        f.v.write_text_atomic("/vendor/dep.rs", "3").await.unwrap();
        let r = glob_files(&f.v, "/", "*.rs", &["*/vendor/*".to_string()]).await.unwrap();
        let matches: Vec<&str> = r["matches"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(matches, vec!["/keep.rs"]);
    }

    #[tokio::test]
    async fn glob_caps_at_one_hundred_and_flags_truncated() {
        let f = fixture();
        for i in 0..105 {
            f.v.write_text_atomic(&format!("/f{i}.log"), "x").await.unwrap();
        }
        let r = glob_files(&f.v, "/", "*.log", &[]).await.unwrap();
        assert_eq!(r["matches"].as_array().unwrap().len(), GLOB_CAP);
        assert_eq!(r["truncated"], true);
    }

    #[tokio::test]
    async fn glob_with_no_match_is_empty_not_an_error() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        let r = glob_files(&f.v, "/", "*.nope", &[]).await.unwrap();
        assert_eq!(r["matches"].as_array().unwrap().len(), 0);
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn grep_content_mode_returns_line_hits() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "alpha\nbeta\nalpha again\n").await.unwrap();
        let r = grep_files(&f.v, "/", "alpha", None, None, true, true, "content", 0, 100)
            .await
            .unwrap();
        let m = r["matches"].as_array().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0]["line"], 1);
        assert_eq!(s(&m[0], "text"), "alpha");
        assert_eq!(m[0]["context"], Value::Null);
        assert_eq!(m[1]["line"], 3);
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn grep_context_lines_are_included() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "one\ntwo\nthree\nfour\n").await.unwrap();
        let r = grep_files(&f.v, "/", "three", None, None, true, true, "content", 1, 100)
            .await
            .unwrap();
        let ctx = r["matches"][0]["context"].as_array().unwrap();
        let ctx: Vec<&str> = ctx.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(ctx, vec!["two", "three", "four"]);
    }

    #[tokio::test]
    async fn grep_files_and_count_modes() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "hit\nhit\n").await.unwrap();
        f.v.write_text_atomic("/b.txt", "nope\n").await.unwrap();
        let files = grep_files(&f.v, "/", "hit", None, None, true, true, "files", 0, 100)
            .await
            .unwrap();
        assert_eq!(files["files"].as_array().unwrap().len(), 1);
        assert_eq!(files["files"][0], "/a.txt");
        assert!(files.get("matches").is_none());

        let count = grep_files(&f.v, "/", "hit", None, None, true, true, "count", 0, 100)
            .await
            .unwrap();
        assert_eq!(count["count"], 2);
        assert_eq!(count["files"], 1);
    }

    #[tokio::test]
    async fn grep_literal_mode_does_not_treat_the_pattern_as_regex() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "a.c\nabc\n").await.unwrap();
        let lit = grep_files(&f.v, "/", "a.c", None, None, false, true, "content", 0, 100)
            .await
            .unwrap();
        assert_eq!(lit["matches"].as_array().unwrap().len(), 1);
        assert_eq!(s(&lit["matches"][0], "text"), "a.c");

        let re = grep_files(&f.v, "/", "a.c", None, None, true, true, "content", 0, 100)
            .await
            .unwrap();
        assert_eq!(re["matches"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn grep_case_insensitivity_and_globs() {
        let f = fixture();
        f.v.write_text_atomic("/a.rs", "Needle\n").await.unwrap();
        f.v.write_text_atomic("/b.txt", "needle\n").await.unwrap();
        let sensitive = grep_files(&f.v, "/", "needle", None, None, true, true, "files", 0, 100)
            .await
            .unwrap();
        assert_eq!(sensitive["files"].as_array().unwrap().len(), 1);

        let insensitive = grep_files(&f.v, "/", "needle", None, None, true, false, "files", 0, 100)
            .await
            .unwrap();
        assert_eq!(insensitive["files"].as_array().unwrap().len(), 2);

        let only_rs = grep_files(&f.v, "/", "needle", Some("*.rs"), None, true, false, "files", 0, 100)
            .await
            .unwrap();
        assert_eq!(only_rs["files"][0], "/a.rs");

        let not_rs = grep_files(&f.v, "/", "needle", None, Some("*.rs"), true, false, "files", 0, 100)
            .await
            .unwrap();
        assert_eq!(not_rs["files"][0], "/b.txt");
    }

    #[tokio::test]
    async fn grep_max_matches_truncates() {
        let f = fixture();
        let body: String = (0..10).map(|_| "hit\n".to_string()).collect();
        f.v.write_text_atomic("/a.txt", &body).await.unwrap();
        let r = grep_files(&f.v, "/", "hit", None, None, true, true, "content", 0, 4)
            .await
            .unwrap();
        assert_eq!(r["matches"].as_array().unwrap().len(), 4);
        assert_eq!(r["truncated"], true);
    }

    #[tokio::test]
    async fn grep_rejects_an_invalid_regex() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x\n").await.unwrap();
        let e = grep_files(&f.v, "/", "a(", None, None, true, true, "content", 0, 10)
            .await
            .unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
    }

    // ── listing / tree ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_dir_hides_dotfiles_by_default_and_sorts_by_name() {
        let f = fixture();
        f.v.write_text_atomic("/d/b.txt", "22").await.unwrap();
        f.v.write_text_atomic("/d/a.txt", "1").await.unwrap();
        f.v.write_text_atomic("/d/.hidden", "x").await.unwrap();
        let r = list_dir(&f.v, "/d", false, "name", false).await.unwrap();
        let names: Vec<&str> = r["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
        assert_eq!(r["total"], 2);
        assert_eq!(s(&r, "path"), "/d");
        assert!(r["entries"][0].get("size").is_none());

        let with_hidden = list_dir(&f.v, "/d", true, "name", false).await.unwrap();
        assert_eq!(with_hidden["total"], 3);
    }

    #[tokio::test]
    async fn list_dir_with_sizes_can_sort_by_size() {
        let f = fixture();
        f.v.write_text_atomic("/d/big.txt", "aaaaa").await.unwrap();
        f.v.write_text_atomic("/d/small.txt", "a").await.unwrap();
        let r = list_dir(&f.v, "/d", false, "size", true).await.unwrap();
        let e = r["entries"].as_array().unwrap();
        assert_eq!(e[0]["name"], "small.txt");
        assert_eq!(e[0]["size"], 1);
        assert_eq!(e[1]["name"], "big.txt");
        assert_eq!(e[1]["size"], 5);
        assert!(e[0]["mtime"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn list_dir_on_a_file_is_invalid_argument() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        let e = list_dir(&f.v, "/a.txt", false, "name", false).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn tree_nests_children_and_prunes_defaults() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "1").await.unwrap();
        f.v.write_text_atomic("/sub/b.txt", "22").await.unwrap();
        f.v.write_text_atomic("/node_modules/junk.js", "x").await.unwrap();
        let r = tree(&f.v, "/", 3, &[], true).await.unwrap();
        assert_eq!(s(&r, "path"), "/");
        assert_eq!(r["truncated"], false);
        let nodes = r["tree"].as_array().unwrap();
        let names: Vec<&str> = nodes.iter().map(|n| n["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["a.txt", "sub"], "node_modules pruned");
        let sub = nodes.iter().find(|n| n["name"] == "sub").unwrap();
        assert_eq!(sub["children"][0]["name"], "b.txt");
        assert_eq!(sub["children"][0]["size"], 2);
    }

    #[tokio::test]
    async fn tree_depth_zero_lists_only_the_top_level() {
        let f = fixture();
        f.v.write_text_atomic("/sub/deep/x.txt", "1").await.unwrap();
        let r = tree(&f.v, "/", 0, &[], false).await.unwrap();
        let nodes = r["tree"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["name"], "sub");
        assert!(nodes[0].get("children").is_none(), "no recursion at depth 0");
    }

    #[tokio::test]
    async fn tree_extra_exclude_patterns_are_added() {
        let f = fixture();
        f.v.write_text_atomic("/keep.txt", "1").await.unwrap();
        f.v.mkdir("/skipme").await.unwrap();
        let r = tree(&f.v, "/", 2, &["skipme".to_string()], false).await.unwrap();
        let names: Vec<&str> = r["tree"].as_array().unwrap().iter().map(|n| n["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["keep.txt"]);
    }

    #[tokio::test]
    async fn tree_reports_truncated_when_the_node_cap_is_reached() {
        let f = fixture();
        for i in 0..(TREE_CAP + 10) {
            f.v.write_text_atomic(&format!("/f{i}.txt"), "x").await.unwrap();
        }
        let r = tree(&f.v, "/", 1, &[], false).await.unwrap();
        assert_eq!(r["truncated"], true);
        // Exactly TREE_CAP nodes are emitted, not TREE_CAP - 1: the reference checked
        // the cap after incrementing and silently dropped the last one.
        assert_eq!(r["tree"].as_array().unwrap().len(), TREE_CAP);
    }

    /// A tree of exactly TREE_CAP entries must come back whole and NOT be flagged as
    /// truncated. The old off-by-one returned one node short and set the flag.
    #[tokio::test]
    async fn tree_at_exactly_the_cap_is_complete_and_not_truncated() {
        let f = fixture();
        for i in 0..TREE_CAP {
            f.v.write_text_atomic(&format!("/f{i}.txt"), "x").await.unwrap();
        }
        let r = tree(&f.v, "/", 1, &[], false).await.unwrap();
        assert_eq!(r["tree"].as_array().unwrap().len(), TREE_CAP);
        assert_eq!(r["truncated"], false, "a tree that fits must not be flagged");
    }

    // ── write ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_creates_parents_and_reports_bytes() {
        let f = fixture();
        let r = write_text(&f.v, &f.s, P, M, "/a/b/c.txt", "hello", false, true).await.unwrap();
        assert_eq!(s(&r, "path"), "/a/b/c.txt");
        assert_eq!(r["bytes_written"], 5);
        assert_eq!(r["overwritten"], false);
        assert_eq!(s(&r, "diff"), "");
        assert!(f.v.is_dir("/a/b").await.unwrap());
        assert_eq!(f.v.read_text("/a/b/c.txt").await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn write_is_no_clobber_by_default() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "old").await.unwrap();
        let e = write_text(&f.v, &f.s, P, M, "/a.txt", "new", false, true).await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);
        assert!(e.message.contains("pass overwrite=true"));
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "old");
    }

    #[tokio::test]
    async fn overwrite_needs_a_prior_read_and_returns_a_diff() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "old\n").await.unwrap();
        let e = write_text(&f.v, &f.s, P, M, "/a.txt", "new\n", true, true).await.unwrap_err();
        assert_eq!(e.code, code::EDIT_WITHOUT_PRIOR_READ);

        f.s.record_read(P, M, "/a.txt");
        let r = write_text(&f.v, &f.s, P, M, "/a.txt", "new\n", true, true).await.unwrap();
        assert_eq!(r["overwritten"], true);
        assert!(s(&r, "diff").contains("-old\n+new\n"));
    }

    #[tokio::test]
    async fn write_charges_the_session_quota() {
        let f = fixture_with(SafetyConfig { write_quota_bytes: 4, ..Default::default() });
        let e = write_text(&f.v, &f.s, P, M, "/a.txt", "12345", false, true).await.unwrap_err();
        assert_eq!(e.code, code::WRITE_QUOTA_EXCEEDED);
        assert!(!f.v.exists("/a.txt").await.unwrap(), "nothing was written");
    }

    #[tokio::test]
    async fn write_then_edit_needs_no_extra_read() {
        let f = fixture();
        write_text(&f.v, &f.s, P, M, "/a.txt", "x\n", false, true).await.unwrap();
        let r = edit_unique(&f.v, &f.s, P, M, "/a.txt", "x", "y", false, false).await.unwrap();
        assert_eq!(r["applied"], true);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "y\n");
    }

    #[tokio::test]
    async fn append_requires_create_for_a_missing_file() {
        let f = fixture();
        let e = append_text(&f.v, &f.s, P, M, "/a.txt", "x", false).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
        assert!(e.message.contains("pass create=true"));

        let r = append_text(&f.v, &f.s, P, M, "/a.txt", "x", true).await.unwrap();
        assert_eq!(r["bytes_appended"], 1);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "x");
    }

    #[tokio::test]
    async fn append_concatenates_and_charges_only_the_delta() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "one\n").await.unwrap();
        let r = append_text(&f.v, &f.s, P, M, "/a.txt", "two\n", false).await.unwrap();
        assert_eq!(r["bytes_appended"], 4);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "one\ntwo\n");
        assert_eq!(f.s.bytes_written(P, M), 4, "only the appended bytes are charged");
    }

    #[tokio::test]
    async fn create_empty_is_no_clobber_unless_exist_ok() {
        let f = fixture();
        let r = create_empty(&f.v, &f.s, P, M, "/e.txt", false).await.unwrap();
        assert_eq!(r["created"], true);
        assert_eq!(f.v.stat("/e.txt").await.unwrap().size, 0);

        let e = create_empty(&f.v, &f.s, P, M, "/e.txt", false).await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);

        let ok = create_empty(&f.v, &f.s, P, M, "/e.txt", true).await.unwrap();
        assert_eq!(ok["created"], false);
    }

    // ── edit ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn edit_replaces_a_unique_occurrence() {
        let f = fixture();
        seed(&f, "/a.txt", "alpha\nbeta\n").await;
        let r = edit_unique(&f.v, &f.s, P, M, "/a.txt", "beta", "gamma", false, false).await.unwrap();
        assert_eq!(r["applied"], true);
        assert!(s(&r, "diff").contains("-beta\n+gamma\n"));
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "alpha\ngamma\n");
    }

    #[tokio::test]
    async fn edit_rejects_an_ambiguous_match() {
        let f = fixture();
        seed(&f, "/a.txt", "x\nx\n").await;
        let e = edit_unique(&f.v, &f.s, P, M, "/a.txt", "x", "y", false, false).await.unwrap_err();
        assert_eq!(e.code, code::AMBIGUOUS_MATCH);
        assert!(e.message.contains("matches 2 sites"));
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "x\nx\n");
    }

    #[tokio::test]
    async fn edit_replace_all_rewrites_every_site() {
        let f = fixture();
        seed(&f, "/a.txt", "x\nx\nx\n").await;
        let r = edit_unique(&f.v, &f.s, P, M, "/a.txt", "x", "y", true, false).await.unwrap();
        assert_eq!(r["applied"], true);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "y\ny\ny\n");
    }

    #[tokio::test]
    async fn edit_no_match_is_an_error() {
        let f = fixture();
        seed(&f, "/a.txt", "alpha\n").await;
        let e = edit_unique(&f.v, &f.s, P, M, "/a.txt", "zeta", "y", false, false).await.unwrap_err();
        assert_eq!(e.code, code::NO_MATCH);
        assert!(e.message.contains("old_string not found"));
    }

    #[tokio::test]
    async fn edit_dry_run_returns_the_diff_without_writing() {
        let f = fixture();
        seed(&f, "/a.txt", "alpha\n").await;
        let r = edit_unique(&f.v, &f.s, P, M, "/a.txt", "alpha", "beta", false, true).await.unwrap();
        assert_eq!(r["applied"], false);
        assert!(s(&r, "diff").contains("+beta"));
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "alpha\n", "untouched");
        assert_eq!(f.s.bytes_written(P, M), 0, "a dry run charges nothing");
    }

    #[tokio::test]
    async fn multi_edit_applies_sequentially_and_counts_edits() {
        let f = fixture();
        seed(&f, "/a.txt", "one two three\n").await;
        let edits = vec![
            json!({"old_string": "one", "new_string": "1"}),
            json!({"old_string": "three", "new_string": "3", "replace_all": false}),
        ];
        let r = multi_edit(&f.v, &f.s, P, M, "/a.txt", &edits, false).await.unwrap();
        assert_eq!(r["applied"], true);
        assert_eq!(r["edits"], 2);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "1 two 3\n");
    }

    #[tokio::test]
    async fn multi_edit_is_all_or_nothing() {
        let f = fixture();
        seed(&f, "/a.txt", "one two\n").await;
        let edits = vec![
            json!({"old_string": "one", "new_string": "1"}),
            json!({"old_string": "absent", "new_string": "x"}),
        ];
        let e = multi_edit(&f.v, &f.s, P, M, "/a.txt", &edits, false).await.unwrap_err();
        assert_eq!(e.code, code::NO_MATCH);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "one two\n", "rolled back");
    }

    #[tokio::test]
    async fn multi_edit_accepts_replace_all_as_a_string() {
        let f = fixture();
        seed(&f, "/a.txt", "x x x\n").await;
        let edits = vec![json!({"old_string": "x", "new_string": "y", "replace_all": "TRUE"})];
        multi_edit(&f.v, &f.s, P, M, "/a.txt", &edits, false).await.unwrap();
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "y y y\n");
    }

    #[tokio::test]
    async fn multi_edit_dry_run_does_not_write() {
        let f = fixture();
        seed(&f, "/a.txt", "a\n").await;
        let edits = vec![json!({"old_string": "a", "new_string": "b"})];
        let r = multi_edit(&f.v, &f.s, P, M, "/a.txt", &edits, true).await.unwrap();
        assert_eq!(r["applied"], false);
        assert!(!s(&r, "diff").is_empty());
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "a\n");
    }

    #[tokio::test]
    async fn multi_edit_needs_a_prior_read() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "a\n").await.unwrap();
        let edits = vec![json!({"old_string": "a", "new_string": "b"})];
        let e = multi_edit(&f.v, &f.s, P, M, "/a.txt", &edits, false).await.unwrap_err();
        assert_eq!(e.code, code::EDIT_WITHOUT_PRIOR_READ);
    }

    #[tokio::test]
    async fn search_replace_swaps_an_exact_block() {
        let f = fixture();
        seed(&f, "/a.txt", "head\nold1\nold2\ntail\n").await;
        let r = search_replace(&f.v, &f.s, P, M, "/a.txt", "old1\nold2\n", "new\n", false)
            .await
            .unwrap();
        assert_eq!(r["applied"], true);
        assert!(!s(&r, "diff").is_empty());
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "head\nnew\ntail\n");
    }

    #[tokio::test]
    async fn search_replace_without_fuzzy_requires_an_exact_block() {
        let f = fixture();
        seed(&f, "/a.txt", "head\nold  1\ntail\n").await;
        let e = search_replace(&f.v, &f.s, P, M, "/a.txt", "old 1\n", "new\n", false)
            .await
            .unwrap_err();
        assert_eq!(e.code, code::NO_MATCH);
        assert!(e.message.contains("search_block not found"));
    }

    #[tokio::test]
    async fn search_replace_fuzzy_tolerates_small_differences() {
        let f = fixture();
        seed(&f, "/a.txt", "head\nold  value here\ntail\n").await;
        let r = search_replace(&f.v, &f.s, P, M, "/a.txt", "old value here\n", "new\n", true)
            .await
            .unwrap();
        assert_eq!(r["applied"], true);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "head\nnew\ntail\n");
    }

    #[tokio::test]
    async fn search_replace_fuzzy_still_refuses_a_hopeless_block() {
        let f = fixture();
        seed(&f, "/a.txt", "aaaa\nbbbb\n").await;
        let e = search_replace(
            &f.v,
            &f.s,
            P,
            M,
            "/a.txt",
            "zzzzzzzzzzzzzzzzzzzzzzzz\n",
            "new\n",
            true,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, code::NO_MATCH);
        assert!(e.message.contains("no fuzzy match"));
    }

    #[tokio::test]
    async fn insert_at_line_puts_content_before_the_line() {
        let f = fixture();
        seed(&f, "/a.txt", "one\ntwo\n").await;
        let r = insert_at_line(&f.v, &f.s, P, M, "/a.txt", 2, "middle").await.unwrap();
        assert_eq!(r["applied"], true);
        assert_eq!(r["line"], 2);
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "one\nmiddle\ntwo\n");
    }

    #[tokio::test]
    async fn insert_at_line_clamps_beyond_the_end_and_before_the_start() {
        let f = fixture();
        seed(&f, "/a.txt", "one\n").await;
        insert_at_line(&f.v, &f.s, P, M, "/a.txt", 999, "last\n").await.unwrap();
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "one\nlast\n");
        insert_at_line(&f.v, &f.s, P, M, "/a.txt", -5, "first").await.unwrap();
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "first\none\nlast\n");
    }

    #[tokio::test]
    async fn insert_at_line_needs_a_prior_read() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "one\n").await.unwrap();
        let e = insert_at_line(&f.v, &f.s, P, M, "/a.txt", 1, "x").await.unwrap_err();
        assert_eq!(e.code, code::EDIT_WITHOUT_PRIOR_READ);
    }

    // ── lifecycle ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mkdir_creates_parents_and_audits() {
        let f = fixture();
        let r = mkdir(&f.v, &f.s, P, M, "/a/b/c", true, true).await.unwrap();
        assert_eq!(s(&r, "path"), "/a/b/c");
        assert_eq!(r["created"], true);
        assert!(f.v.is_dir("/a/b/c").await.unwrap());
        let log = f.s.audit(P, M);
        assert_eq!(log.last().unwrap().op, "mkdir");
    }

    #[tokio::test]
    async fn mkdir_without_parents_on_an_existing_path() {
        let f = fixture();
        f.v.mkdir("/d").await.unwrap();
        let e = mkdir(&f.v, &f.s, P, M, "/d", false, false).await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);
        let ok = mkdir(&f.v, &f.s, P, M, "/d", false, true).await.unwrap();
        assert_eq!(ok["created"], true);
    }

    #[tokio::test]
    async fn delete_soft_moves_into_the_trash() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        let r = delete_path(&f.v, &f.s, P, M, "/a.txt", false, true).await.unwrap();
        assert_eq!(r["trashed"], true);
        let dst = s(&r, "trash_path");
        assert!(dst.starts_with("/.mcp_trash/"), "got {dst}");
        assert!(dst.ends_with("__a.txt"), "got {dst}");
        assert!(!f.v.exists("/a.txt").await.unwrap());
        assert_eq!(f.v.read_text(&dst).await.unwrap(), "x");
    }

    #[tokio::test]
    async fn hard_delete_is_refused_unless_configured() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        let e = delete_path(&f.v, &f.s, P, M, "/a.txt", false, false).await.unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
        assert!(f.v.exists("/a.txt").await.unwrap());
    }

    #[tokio::test]
    async fn hard_delete_works_when_allowed() {
        let f = fixture_with(SafetyConfig { allow_hard_delete: true, ..Default::default() });
        f.v.write_text_atomic("/d/a.txt", "x").await.unwrap();
        let file = delete_path(&f.v, &f.s, P, M, "/d/a.txt", false, false).await.unwrap();
        assert_eq!(file["trashed"], false);
        assert_eq!(file["trash_path"], Value::Null);
        assert!(!f.v.exists("/d/a.txt").await.unwrap());

        f.v.write_text_atomic("/d/sub/b.txt", "y").await.unwrap();
        delete_path(&f.v, &f.s, P, M, "/d", true, false).await.unwrap();
        assert!(!f.v.exists("/d/sub/b.txt").await.unwrap());
        assert!(!f.v.exists("/d").await.unwrap());
    }

    #[tokio::test]
    async fn delete_a_directory_needs_recursive() {
        let f = fixture();
        f.v.write_text_atomic("/d/a.txt", "x").await.unwrap();
        let e = delete_path(&f.v, &f.s, P, M, "/d", false, true).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("pass recursive=true"));
    }

    #[tokio::test]
    async fn delete_a_missing_path_is_not_found() {
        let f = fixture();
        let e = delete_path(&f.v, &f.s, P, M, "/nope", false, true).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn move_renames_and_is_no_clobber() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "x").await.unwrap();
        f.v.write_text_atomic("/b.txt", "y").await.unwrap();
        let e = move_path(&f.v, &f.s, P, M, "/a.txt", "/b.txt", false).await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);

        let r = move_path(&f.v, &f.s, P, M, "/a.txt", "/c.txt", false).await.unwrap();
        assert_eq!(s(&r, "source"), "/a.txt");
        assert_eq!(s(&r, "destination"), "/c.txt");
        assert!(!f.v.exists("/a.txt").await.unwrap());
        assert_eq!(f.v.read_text("/c.txt").await.unwrap(), "x");
    }

    #[tokio::test]
    async fn move_a_missing_source_is_not_found() {
        let f = fixture();
        let e = move_path(&f.v, &f.s, P, M, "/nope", "/x", false).await.unwrap_err();
        assert_eq!(e.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn copy_a_file_charges_quota_and_materializes_parents() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "hello").await.unwrap();
        let r = copy_path(&f.v, &f.s, P, M, "/a.txt", "/deep/b.txt", false, false).await.unwrap();
        assert_eq!(s(&r, "source"), "/a.txt");
        assert_eq!(s(&r, "destination"), "/deep/b.txt");
        assert_eq!(f.v.read_text("/deep/b.txt").await.unwrap(), "hello");
        assert_eq!(f.s.bytes_written(P, M), 5);
    }

    #[tokio::test]
    async fn copy_a_directory_needs_recursive_then_copies_the_tree() {
        let f = fixture();
        f.v.write_text_atomic("/s/a.txt", "1").await.unwrap();
        f.v.write_text_atomic("/s/sub/b.txt", "2").await.unwrap();
        let e = copy_path(&f.v, &f.s, P, M, "/s", "/t", false, false).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("pass recursive=true"));

        copy_path(&f.v, &f.s, P, M, "/s", "/t", false, true).await.unwrap();
        assert_eq!(f.v.read_text("/t/a.txt").await.unwrap(), "1");
        assert_eq!(f.v.read_text("/t/sub/b.txt").await.unwrap(), "2");
    }

    #[tokio::test]
    async fn copy_is_no_clobber_and_reports_a_missing_source() {
        let f = fixture();
        f.v.write_text_atomic("/a.txt", "1").await.unwrap();
        f.v.write_text_atomic("/b.txt", "2").await.unwrap();
        let clobber = copy_path(&f.v, &f.s, P, M, "/a.txt", "/b.txt", false, false).await.unwrap_err();
        assert_eq!(clobber.code, code::NO_CLOBBER);

        let missing = copy_path(&f.v, &f.s, P, M, "/nope", "/x", false, false).await.unwrap_err();
        assert_eq!(missing.code, code::NOT_FOUND);

        copy_path(&f.v, &f.s, P, M, "/a.txt", "/b.txt", true, false).await.unwrap();
        assert_eq!(f.v.read_text("/b.txt").await.unwrap(), "1");
    }

    #[tokio::test]
    async fn mutations_land_in_the_audit_log_in_order() {
        let f = fixture();
        write_text(&f.v, &f.s, P, M, "/a.txt", "one\n", false, true).await.unwrap();
        edit_unique(&f.v, &f.s, P, M, "/a.txt", "one", "two", false, false).await.unwrap();
        move_path(&f.v, &f.s, P, M, "/a.txt", "/b.txt", false).await.unwrap();
        let ops: Vec<String> = f.s.audit(P, M).into_iter().map(|e| e.op).collect();
        assert_eq!(ops, vec!["write", "edit", "move"]);
    }

    #[tokio::test]
    async fn soft_delete_of_a_directory_moves_the_whole_subtree() {
        let f = fixture();
        f.v.write_text_atomic("/d/sub/a.txt", "x").await.unwrap();
        let r = delete_path(&f.v, &f.s, P, M, "/d", true, true).await.unwrap();
        let dst = s(&r, "trash_path");
        assert!(dst.ends_with("__d"), "got {dst}");
        assert!(!f.v.exists("/d").await.unwrap());
        assert_eq!(f.v.read_text(&format!("{dst}/sub/a.txt")).await.unwrap(), "x");
    }

    #[tokio::test]
    async fn overwriting_with_identical_content_yields_an_empty_diff() {
        let f = fixture();
        seed(&f, "/a.txt", "same\n").await;
        let r = write_text(&f.v, &f.s, P, M, "/a.txt", "same\n", true, true).await.unwrap();
        assert_eq!(r["overwritten"], true);
        assert_eq!(s(&r, "diff"), "", "no change means no diff");
    }

    #[tokio::test]
    async fn read_window_with_a_negative_offset_starts_at_the_top() {
        let f = fixture();
        seed(&f, "/a.txt", "a\nb\n").await;
        let r = read_window(&f.v, &f.s, P, M, "/a.txt", -2, 4, true).await.unwrap();
        // The window is clamped but the numbering still follows the requested offset.
        assert_eq!(s(&r, "content"), "-1\ta\n0\tb");
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn multi_edit_with_no_edits_is_a_no_op_diff() {
        let f = fixture();
        seed(&f, "/a.txt", "a\n").await;
        let r = multi_edit(&f.v, &f.s, P, M, "/a.txt", &[], false).await.unwrap();
        assert_eq!(r["edits"], 0);
        assert_eq!(s(&r, "diff"), "");
        assert_eq!(f.v.read_text("/a.txt").await.unwrap(), "a\n");
    }

    #[tokio::test]
    async fn insert_into_an_empty_file() {
        let f = fixture();
        seed(&f, "/e.txt", "").await;
        insert_at_line(&f.v, &f.s, P, M, "/e.txt", 1, "first").await.unwrap();
        assert_eq!(f.v.read_text("/e.txt").await.unwrap(), "first\n");
    }

    #[tokio::test]
    async fn edit_with_an_empty_old_string_never_matches() {
        let f = fixture();
        seed(&f, "/a.txt", "content\n").await;
        let e = edit_unique(&f.v, &f.s, P, M, "/a.txt", "", "x", false, false).await.unwrap_err();
        assert_eq!(e.code, code::NO_MATCH);
    }

    #[tokio::test]
    async fn edit_preserves_unicode_around_the_replacement() {
        let f = fixture();
        seed(&f, "/u.txt", "héllo ✅ wörld\n").await;
        edit_unique(&f.v, &f.s, P, M, "/u.txt", "✅", "❌", false, false).await.unwrap();
        assert_eq!(f.v.read_text("/u.txt").await.unwrap(), "héllo ❌ wörld\n");
    }

    #[tokio::test]
    async fn grep_and_glob_are_scoped_to_the_root() {
        let f = fixture();
        f.v.write_text_atomic("/in/a.txt", "needle\n").await.unwrap();
        f.v.write_text_atomic("/out/b.txt", "needle\n").await.unwrap();
        let g = glob_files(&f.v, "/in", "*.txt", &[]).await.unwrap();
        assert_eq!(g["matches"].as_array().unwrap(), &vec![json!("/in/a.txt")]);
        let r = grep_files(&f.v, "/in", "needle", None, None, true, true, "files", 0, 100)
            .await
            .unwrap();
        assert_eq!(r["files"].as_array().unwrap(), &vec![json!("/in/a.txt")]);
    }

    #[tokio::test]
    async fn copy_tree_over_an_existing_destination_needs_overwrite() {
        let f = fixture();
        f.v.write_text_atomic("/s/a.txt", "new").await.unwrap();
        f.v.write_text_atomic("/t/a.txt", "old").await.unwrap();
        let e = copy_path(&f.v, &f.s, P, M, "/s", "/t", false, true).await.unwrap_err();
        assert_eq!(e.code, code::NO_CLOBBER);
        copy_path(&f.v, &f.s, P, M, "/s", "/t", true, true).await.unwrap();
        assert_eq!(f.v.read_text("/t/a.txt").await.unwrap(), "new");
    }

    // ── helper units ─────────────────────────────────────────────────────────

    #[test]
    fn fnmatch_star_crosses_slashes_like_python() {
        // This is why fs.glob does not use util::text::glob_match.
        assert!(crate::util::text::Fnmatch::new("*.rs").is_match("/src/main.rs"));
        assert!(crate::util::text::Fnmatch::new("/src/*").is_match("/src/a/b.rs"));
        assert!(!crate::util::text::Fnmatch::new("*.rs").is_match("/src/main.txt"));
        assert!(crate::util::text::Fnmatch::new("a?c").is_match("abc"));
        assert!(crate::util::text::Fnmatch::new("[abc]x").is_match("bx"));
        assert!(!crate::util::text::Fnmatch::new("[!abc]x").is_match("bx"));
        assert!(crate::util::text::Fnmatch::new("[!abc]x").is_match("dx"));
        // an unterminated class degrades to a literal bracket
        assert!(crate::util::text::Fnmatch::new("a[bc").is_match("a[bc"));
    }

    #[test]
    fn slice_clamps_every_out_of_range_bound() {
        let lines: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(slice(&lines, 0, 2), &lines[0..2]);
        assert_eq!(slice(&lines, -5, 99), &lines[..]);
        assert_eq!(slice(&lines, 2, 1).len(), 0);
        assert_eq!(slice(&lines, 5, 9).len(), 0);
    }

    #[test]
    fn number_lines_starts_where_asked() {
        let lines: Vec<String> = vec!["x".into(), "y".into()];
        assert_eq!(number_lines(&lines, 10), "10\tx\n11\ty");
        assert_eq!(number_lines(&[], 1), "");
    }

    #[test]
    fn mode_helpers_render_posix_values() {
        assert_eq!(kind_of(0o100_644), "file");
        assert_eq!(kind_of(0o040_755), "dir");
        assert_eq!(kind_of(0o120_777), "symlink");
        assert_eq!(kind_of(0), "other");
        assert_eq!(oct_permissions(0o100_644), "0o644");
        assert_eq!(oct_permissions(0o040_755), "0o755");
    }

    #[test]
    fn mime_guess_is_case_insensitive_and_optional() {
        assert_eq!(mime_guess("/a.PNG"), Some("image/png"));
        assert_eq!(mime_guess("/a.md"), Some("text/markdown"));
        assert_eq!(mime_guess("/noext"), None);
        assert_eq!(mime_guess("/a.unknownext"), None);
    }

    #[test]
    fn similarity_ratio_bounds() {
        assert_eq!(similarity_ratio("", ""), 1.0);
        assert_eq!(similarity_ratio("abc", "abc"), 1.0);
        assert_eq!(similarity_ratio("abc", "xyz"), 0.0);
        let r = similarity_ratio("old value", "old  value");
        assert!(r > FUZZY_THRESHOLD, "got {r}");
    }

    #[test]
    fn to_bool_matches_the_csharp_coercion() {
        assert!(to_bool(Some(&json!(true))));
        assert!(!to_bool(Some(&json!(false))));
        assert!(to_bool(Some(&json!("True"))));
        assert!(!to_bool(Some(&json!("yes"))));
        assert!(!to_bool(Some(&json!(1))));
        assert!(!to_bool(None));
    }

    #[test]
    fn count_occurrences_is_non_overlapping_and_rejects_empty() {
        assert_eq!(count_occurrences("aaaa", "aa"), 2);
        assert_eq!(count_occurrences("abc", ""), 0);
        assert_eq!(count_occurrences("abc", "z"), 0);
    }

    /// `overwrite: true` used to be accepted and then ignored: the metadata store
    /// refuses to rename onto an existing path, so the move always failed with
    /// NO_CLOBBER and the flag was dead.
    #[tokio::test]
    async fn move_with_overwrite_replaces_the_destination() {
        let f = fixture();
        f.v.write_text_atomic("/from.txt", "new").await.unwrap();
        f.v.write_text_atomic("/onto.txt", "old").await.unwrap();

        let e = move_path(&f.v, &f.s, P, M, "/from.txt", "/onto.txt", false)
            .await
            .unwrap_err();
        assert_eq!(e.code, crate::errors::code::NO_CLOBBER);

        move_path(&f.v, &f.s, P, M, "/from.txt", "/onto.txt", true).await.unwrap();
        assert_eq!(f.v.read_text("/onto.txt").await.unwrap(), "new");
        assert!(!f.v.exists("/from.txt").await.unwrap());
    }

    /// Overwriting a directory destination clears the whole subtree first.
    #[tokio::test]
    async fn move_with_overwrite_replaces_a_directory_destination() {
        let f = fixture();
        f.v.write_text_atomic("/src/a.txt", "keep").await.unwrap();
        f.v.write_text_atomic("/dst/stale.txt", "gone").await.unwrap();

        move_path(&f.v, &f.s, P, M, "/src", "/dst", true).await.unwrap();
        assert_eq!(f.v.read_text("/dst/a.txt").await.unwrap(), "keep");
        assert!(!f.v.exists("/dst/stale.txt").await.unwrap(), "stale entry removed");
    }

    // ── V4A patch internals (moved here with the engine) ──────────────────

    /// Running out of lines without `*** End Patch` is an error. A trailing empty
    /// line instead reports `unexpected patch line`, exactly like the C#.
    #[test]
    fn parse_patch_requires_the_end_marker() {
        let err = parse_patch("*** Begin Patch\n*** Delete File: /a").unwrap_err();
        assert!(err.message.contains("End Patch"), "got {}", err.message);

        let trailing = parse_patch("*** Begin Patch\n*** Delete File: /a\n").unwrap_err();
        assert!(trailing.message.contains("unexpected patch line"));
    }

    #[test]
    fn parse_patch_rejects_an_unexpected_line() {
        let err = parse_patch("*** Begin Patch\ngarbage\n*** End Patch\n").unwrap_err();
        assert!(err.message.contains("unexpected patch line"));
    }

    /// Context lines frame the change: before it when the hunk is still empty,
    /// after it once a +/- line has been seen.
    #[test]
    fn classify_line_splits_context_around_the_change() {
        let mut h = Hunk::default();
        classify_line(" ctx1", &mut h);
        classify_line("-old", &mut h);
        classify_line("+new", &mut h);
        classify_line(" ctx2", &mut h);
        assert_eq!(h.context_before, vec!["ctx1"]);
        assert_eq!(h.removed, vec!["old"]);
        assert_eq!(h.added, vec!["new"]);
        assert_eq!(h.context_after, vec!["ctx2"]);
    }

    /// A hunk with no context and no removals replaces the whole file.
    #[test]
    fn apply_update_with_an_empty_old_block_replaces_everything() {
        let mut op = FileOp::new(OpKind::Update, "/a.txt".into());
        let mut hunk = Hunk::default();
        hunk.added.push("only".into());
        op.hunks.push(hunk);
        assert_eq!(apply_update("whatever", &op).unwrap(), "only");
    }

    /// CRLF input parses like LF: the parser trims trailing carriage returns.
    #[test]
    fn parse_patch_tolerates_crlf() {
        let ops = parse_patch("*** Begin Patch\r\n*** Delete File: /a.txt\r\n*** End Patch\r\n").unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind, OpKind::Delete);
        assert_eq!(ops[0].path, "/a.txt");
    }
}
