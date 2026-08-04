//! Edit family: `fs.edit`, `fs.multi_edit`, `fs.search_replace`,
//! `fs.insert_at_line`, `fs.apply_patch` (V4A).
//!
//! Port of the C# `Tools/EditTools.cs`. The first four delegate straight to the
//! engine; `fs.apply_patch` also carries the V4A parser/applier (port of the C#
//! `Core/PatchV4A.cs`), because the patch envelope is a tool level concern: the
//! engine only exposes the primitives it drives (read, write, delete, rename).

use crate::core::fs_ops;
use crate::errors::{Result, ToolError};
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm, volume};
use serde_json::{Value, json};

/// The `edits` items schema, byte for byte what the C# SDK generates from
/// `List<EditSpec>` (property order included, and no `required` list).
const EDIT_ITEMS: &str = r#"{"type":"object","properties":{"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}}}"#;

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.edit", "Replace a unique string; dry_run returns the diff.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_str("old_string", "Exact text to find; must be unique unless replace_all is set.")
            .req_str("new_string", "Replacement text substituted for old_string.")
            .opt_bool(
                "replace_all",
                false,
                "Replace every occurrence instead of requiring a unique match.",
            )
            .opt_bool("dry_run", false, "Return the diff without writing changes."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::edit_unique(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("old_string")?,
                &a.str("new_string")?,
                a.bool_or("replace_all", false),
                a.bool_or("dry_run", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.multi_edit", "Apply several edits atomically (all or nothing).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_obj_array(
                "edits",
                "Ordered edits (old_string, new_string, replace_all) applied atomically.",
                EDIT_ITEMS,
            )
            .opt_bool("dry_run", false, "Return the diff without writing changes."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let edits = edit_specs(&a)?;
            fs_ops::multi_edit(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &edits,
                a.bool_or("dry_run", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.search_replace", "Replace a multi-line block (optional fuzzy match).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_str("search_block", "Multi-line block of text to locate.")
            .req_str("replace_block", "Multi-line block that replaces search_block.")
            .opt_bool("fuzzy", false, "Allow whitespace tolerant (fuzzy) matching of search_block."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::search_replace(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("search_block")?,
                &a.str("replace_block")?,
                a.bool_or("fuzzy", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.insert_at_line", "Insert content before a 1-based line number.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_int("line", "1-based line number to insert content before.")
            .req_str("content", "Text content to insert."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::insert_at_line(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int("line")?,
                &a.str("content")?,
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.apply_patch", "Apply a multi-file V4A patch within one volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("patch_text", "Multi-file V4A patch text to apply within the volume."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            apply_patch(&ctx, &client, &mount, &a.str("patch_text")?).await
        }),
    );
}

/// Take `edits` as raw JSON: the engine reads `old_string` / `new_string` /
/// `replace_all` per entry with the same defaults the C# `EditSpec` has, so no
/// intermediate struct is needed.
fn edit_specs(a: &crate::mcp::Args) -> Result<Vec<Value>> {
    match a.raw("edits") {
        Some(Value::Array(v)) => Ok(v.clone()),
        Some(_) => Err(ToolError::invalid_argument("argument 'edits' must be an array of objects")),
        None => Err(ToolError::invalid_argument("missing required argument 'edits'")),
    }
}

// ─────────────────────────────────────────────────────── apply_patch (V4A) ────

/// Apply every operation of a V4A patch, in order. Port of the C#
/// `FsOps.ApplyPatch`. Key: `files`, one entry per touched path.
async fn apply_patch(
    ctx: &crate::mcp::registry::ToolCtx,
    client: &crate::storage::VolumeClient,
    mount: &str,
    patch_text: &str,
) -> Result<Value> {
    let safety = &ctx.state.safety;
    let person = &ctx.person;
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

fn replace_first(haystack: &str, search: &str, replace: &str) -> String {
    match haystack.find(search) {
        None => haystack.to_string(),
        Some(i) => format!("{}{replace}{}", &haystack[..i], &haystack[i + search.len()..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness};

    const NAMES: &[&str] = &[
        "fs.edit",
        "fs.multi_edit",
        "fs.search_replace",
        "fs.insert_at_line",
        "fs.apply_patch",
    ];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_edit_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.edit",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "old_string":{"description":"Exact text to find; must be unique unless replace_all is set.","type":"string"},
                 "new_string":{"description":"Replacement text substituted for old_string.","type":"string"},
                 "replace_all":{"description":"Replace every occurrence instead of requiring a unique match.","type":"boolean","default":false},
                 "dry_run":{"description":"Return the diff without writing changes.","type":"boolean","default":false}},
               "required":["mount_id","path","old_string","new_string"]}"#,
        );
        assert_description(register, "fs.edit", "Replace a unique string; dry_run returns the diff.");
    }

    /// The nested `edits` items schema is the part most likely to drift.
    #[test]
    fn fs_multi_edit_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.multi_edit",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "edits":{"description":"Ordered edits (old_string, new_string, replace_all) applied atomically.","type":"array",
                          "items":{"type":"object","properties":{"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}}}},
                 "dry_run":{"description":"Return the diff without writing changes.","type":"boolean","default":false}},
               "required":["mount_id","path","edits"]}"#,
        );
    }

    #[test]
    fn fs_apply_patch_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.apply_patch",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "patch_text":{"description":"Multi-file V4A patch text to apply within the volume.","type":"string"}},
               "required":["mount_id","patch_text"]}"#,
        );
    }

    #[tokio::test]
    async fn edit_applies_and_returns_a_diff() {
        let h = harness().await;
        h.seed("/a.txt", "hello\nworld\n").await;
        let r = h
            .call(
                "fs.edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/a.txt",
                                   "old_string": "world", "new_string": "rust"}),
            )
            .await
            .unwrap();
        assert_eq!(r["applied"], true);
        assert!(r["diff"].as_str().unwrap().contains("+rust"));
    }

    #[tokio::test]
    async fn edit_without_a_prior_read_is_blocked() {
        let h = harness().await;
        h.client().await.write_text_atomic("/g.txt", "x\n").await.unwrap();
        let err = h
            .call(
                "fs.edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/g.txt",
                                   "old_string": "x", "new_string": "y"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::EDIT_WITHOUT_PRIOR_READ);
    }

    #[tokio::test]
    async fn multi_edit_is_atomic_and_counts_edits() {
        let h = harness().await;
        h.seed("/m.txt", "one\ntwo\nthree\n").await;
        let r = h
            .call(
                "fs.multi_edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/m.txt", "edits": [
                    {"old_string": "one", "new_string": "1"},
                    {"old_string": "two", "new_string": "2"}]}),
            )
            .await
            .unwrap();
        assert_eq!(r["edits"], 2);
        assert_eq!(r["applied"], true);

        // A batch whose second edit cannot resolve writes nothing.
        let err = h
            .call(
                "fs.multi_edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/m.txt", "edits": [
                    {"old_string": "1", "new_string": "uno"},
                    {"old_string": "absent", "new_string": "x"}]}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_MATCH);
        let after = h.client().await.read_text("/m.txt").await.unwrap();
        assert_eq!(after, "1\n2\nthree\n");
    }

    #[tokio::test]
    async fn multi_edit_rejects_a_non_array_edits() {
        let h = harness().await;
        h.seed("/m.txt", "one\n").await;
        let err = h
            .call(
                "fs.multi_edit",
                serde_json::json!({"mount_id": MOUNT, "path": "/m.txt", "edits": "nope"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn search_replace_and_insert_at_line_mutate_the_file() {
        let h = harness().await;
        h.seed("/s.txt", "alpha\nbeta\n").await;
        let sr = h
            .call(
                "fs.search_replace",
                serde_json::json!({"mount_id": MOUNT, "path": "/s.txt",
                                   "search_block": "beta", "replace_block": "gamma"}),
            )
            .await
            .unwrap();
        assert_eq!(sr["applied"], true);

        let ins = h
            .call(
                "fs.insert_at_line",
                serde_json::json!({"mount_id": MOUNT, "path": "/s.txt", "line": 1, "content": "zero"}),
            )
            .await
            .unwrap();
        assert_eq!(ins["line"], 1);
        assert_eq!(h.client().await.read_text("/s.txt").await.unwrap(), "zero\nalpha\ngamma\n");
    }

    #[tokio::test]
    async fn apply_patch_adds_updates_and_deletes() {
        let h = harness().await;
        h.seed("/keep.txt", "one\ntwo\n").await;
        h.seed("/gone.txt", "bye\n").await;
        let patch = "*** Begin Patch\n\
                     *** Add File: /new.txt\n\
                     +fresh\n\
                     *** Update File: /keep.txt\n\
                     @@\n\
                     -two\n\
                     +TWO\n\
                     *** Delete File: /gone.txt\n\
                     *** End Patch\n";
        let r = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": patch}))
            .await
            .unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], serde_json::json!({"path": "/new.txt", "op": "add"}));
        assert_eq!(files[1], serde_json::json!({"path": "/keep.txt", "op": "update"}));
        assert_eq!(files[2], serde_json::json!({"path": "/gone.txt", "op": "delete"}));

        let client = h.client().await;
        assert_eq!(client.read_text("/new.txt").await.unwrap(), "fresh");
        assert_eq!(client.read_text("/keep.txt").await.unwrap(), "one\nTWO\n");
        assert!(!client.exists("/gone.txt").await.unwrap());
    }

    #[tokio::test]
    async fn apply_patch_honours_move_to() {
        let h = harness().await;
        h.seed("/old.txt", "body\n").await;
        let patch = "*** Begin Patch\n\
                     *** Update File: /old.txt\n\
                     *** Move to: /moved.txt\n\
                     @@\n\
                     -body\n\
                     +new body\n\
                     *** End Patch\n";
        let r = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": patch}))
            .await
            .unwrap();
        assert_eq!(r["files"][0]["moved_to"], "/moved.txt");
        assert_eq!(h.client().await.read_text("/moved.txt").await.unwrap(), "new body\n");
    }

    #[tokio::test]
    async fn apply_patch_rejects_a_bad_envelope_and_a_stale_hunk() {
        let h = harness().await;
        let err = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);

        h.seed("/a.txt", "one\n").await;
        let stale = "*** Begin Patch\n*** Update File: /a.txt\n@@\n-absent\n+x\n*** End Patch\n";
        let err = h
            .call("fs.apply_patch", serde_json::json!({"mount_id": MOUNT, "patch_text": stale}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_MATCH);
    }

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

    #[test]
    fn replace_first_only_touches_the_first_occurrence() {
        assert_eq!(replace_first("a b a", "a", "z"), "z b a");
        assert_eq!(replace_first("abc", "zz", "y"), "abc");
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
