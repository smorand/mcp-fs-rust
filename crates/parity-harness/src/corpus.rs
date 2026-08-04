//! The parity corpus: a deterministic script of MCP and REST calls replayed
//! against both servers.
//!
//! A step is either a setup mutation (whose response we still record) or a probe.
//! Order matters: later steps depend on earlier ones (a read after a write, an
//! edit after a read so the read-before-write guard is satisfied).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Step {
    /// A JSON-RPC call to the MCP endpoint.
    Mcp { label: String, method: String, params: Value },
    /// A REST call to the /api/fs data plane.
    Rest {
        label: String,
        method: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Value>,
    },
    /// A plain GET with no auth (health, swagger).
    Public { label: String, path: String },
}

impl Step {
    pub fn label(&self) -> &str {
        match self {
            Step::Mcp { label, .. } | Step::Rest { label, .. } | Step::Public { label, .. } => label,
        }
    }
}

fn mcp(label: &str, method: &str, params: Value) -> Step {
    Step::Mcp { label: label.into(), method: method.into(), params }
}

fn call(label: &str, name: &str, args: Value) -> Step {
    mcp(label, "tools/call", json!({"name": name, "arguments": args}))
}

fn rest_get(label: &str, path: &str) -> Step {
    Step::Rest { label: label.into(), method: "GET".into(), path: path.into(), body: None }
}

fn rest_post(label: &str, path: &str, body: Value) -> Step {
    Step::Rest {
        label: label.into(),
        method: "POST".into(),
        path: path.into(),
        body: Some(body),
    }
}

/// Build the corpus. `project` is provisioned by the harness before replay, so
/// every step below can assume it exists and that the caller owns it.
pub fn build(project: &str) -> Vec<Step> {
    let p = project;
    let mut s = Vec::new();

    // ── protocol surface ─────────────────────────────────────────────────────
    s.push(Step::Public { label: "health".into(), path: "/health".into() });
    s.push(mcp(
        "initialize",
        "initialize",
        json!({"protocolVersion":"2024-11-05","capabilities":{},
               "clientInfo":{"name":"parity-harness","version":"1"}}),
    ));
    s.push(mcp("tools_list", "tools/list", json!({})));
    s.push(mcp(
        "unknown_method",
        "does/not/exist",
        json!({}),
    ));
    s.push(call("unknown_tool", "fs.nope", json!({})));

    // ── admin surface ────────────────────────────────────────────────────────
    s.push(call("admin_list_projects", "admin.list_projects", json!({})));
    s.push(call("admin_list_all_projects", "admin.list_all_projects", json!({})));
    s.push(call("admin_list_users", "admin.list_users", json!({})));
    s.push(call("admin_list_members", "admin.list_members", json!({"project_id": p})));
    // validation boundaries
    s.push(call("create_bad_short", "admin.create_project", json!({"project_id":"ab","owner":"x@t.c"})));
    s.push(call(
        "create_bad_long",
        "admin.create_project",
        json!({"project_id":"a".repeat(33),"owner":"x@t.c"}),
    ));
    s.push(call("create_bad_hyphen", "admin.create_project", json!({"project_id":"-abc","owner":"x@t.c"})));
    s.push(call("create_duplicate", "admin.create_project", json!({"project_id":p,"owner":"x@t.c"})));

    // ── write / read round trip ──────────────────────────────────────────────
    s.push(call("write_a", "fs.write", json!({"mount_id":p,"path":"/a.txt","content":"hello\nworld\n"})));
    s.push(call("write_a_noclobber", "fs.write", json!({"mount_id":p,"path":"/a.txt","content":"x"})));
    s.push(call(
        "write_a_overwrite",
        "fs.write",
        json!({"mount_id":p,"path":"/a.txt","content":"hello\nworld\n","overwrite":true}),
    ));
    s.push(call("read_a", "fs.read", json!({"mount_id":p,"path":"/a.txt"})));
    s.push(call("read_a_plain", "fs.read", json!({"mount_id":p,"path":"/a.txt","line_numbered":false})));
    s.push(call("read_a_paged", "fs.read", json!({"mount_id":p,"path":"/a.txt","offset_lines":1,"limit_lines":1})));
    s.push(call("read_bytes_a", "fs.read_bytes", json!({"mount_id":p,"path":"/a.txt"})));
    s.push(call("read_bytes_range", "fs.read_bytes", json!({"mount_id":p,"path":"/a.txt","offset_bytes":2,"length_bytes":3})));
    s.push(call("read_lines_a", "fs.read_lines", json!({"mount_id":p,"path":"/a.txt","start_line":1,"end_line":2})));
    s.push(call("head_a", "fs.head", json!({"mount_id":p,"path":"/a.txt","lines":1})));
    s.push(call("tail_a", "fs.tail", json!({"mount_id":p,"path":"/a.txt","lines":1})));
    s.push(call("count_lines_a", "fs.count_lines", json!({"mount_id":p,"path":"/a.txt"})));
    s.push(call("hash_a_sha256", "fs.hash", json!({"mount_id":p,"path":"/a.txt"})));
    s.push(call("hash_a_md5", "fs.hash", json!({"mount_id":p,"path":"/a.txt","algo":"md5"})));
    s.push(call("hash_a_sha1", "fs.hash", json!({"mount_id":p,"path":"/a.txt","algo":"sha1"})));
    s.push(call("exists_a", "fs.exists", json!({"mount_id":p,"path":"/a.txt"})));
    s.push(call("exists_missing", "fs.exists", json!({"mount_id":p,"path":"/nope.txt"})));

    // ── empty file: no blob, sha256 null ────────────────────────────────────
    s.push(call("create_empty", "fs.create_empty", json!({"mount_id":p,"path":"/empty.txt"})));
    s.push(call("read_empty", "fs.read", json!({"mount_id":p,"path":"/empty.txt"})));
    s.push(call("stat_empty", "fs.stat", json!({"mount_id":p,"path":"/empty.txt"})));

    // ── edit paths (read first so the guard passes) ──────────────────────────
    s.push(call("edit_dry_run", "fs.edit", json!({"mount_id":p,"path":"/a.txt","old_string":"world","new_string":"rust","dry_run":true})));
    s.push(call("edit_apply", "fs.edit", json!({"mount_id":p,"path":"/a.txt","old_string":"world","new_string":"rust"})));
    s.push(call("read_after_edit", "fs.read", json!({"mount_id":p,"path":"/a.txt"})));
    s.push(call("edit_no_match", "fs.edit", json!({"mount_id":p,"path":"/a.txt","old_string":"absent","new_string":"x"})));
    s.push(call("write_dup", "fs.write", json!({"mount_id":p,"path":"/dup.txt","content":"dup dup dup"})));
    s.push(call("read_dup", "fs.read", json!({"mount_id":p,"path":"/dup.txt"})));
    s.push(call("edit_ambiguous", "fs.edit", json!({"mount_id":p,"path":"/dup.txt","old_string":"dup","new_string":"x"})));
    s.push(call("edit_replace_all", "fs.edit", json!({"mount_id":p,"path":"/dup.txt","old_string":"dup","new_string":"x","replace_all":true})));
    s.push(call("append_a", "fs.append", json!({"mount_id":p,"path":"/a.txt","content":"tail\n"})));
    s.push(call("insert_at_line", "fs.insert_at_line", json!({"mount_id":p,"path":"/a.txt","line":1,"content":"first\n"})));
    s.push(call("read_after_insert", "fs.read", json!({"mount_id":p,"path":"/a.txt"})));

    // an edit with no prior read in this session must trip the guard
    s.push(call("write_guard", "fs.write", json!({"mount_id":p,"path":"/guard.txt","content":"g"})));
    s.push(call("edit_without_read", "fs.edit", json!({"mount_id":p,"path":"/guard.txt","old_string":"g","new_string":"h"})));

    // ── structure: dirs, listing, tree, glob, grep ───────────────────────────
    s.push(call("mkdir_src", "fs.mkdir", json!({"mount_id":p,"path":"/src"})));
    s.push(call("mkdir_existing", "fs.mkdir", json!({"mount_id":p,"path":"/src"})));
    s.push(call("write_py", "fs.write", json!({"mount_id":p,"path":"/src/app.py","content":"def hello(name):\n    total = 1\n    return total\n"})));
    s.push(call("write_rs", "fs.write", json!({"mount_id":p,"path":"/src/lib.rs","content":"fn hello() -> u32 { 1 }\n"})));
    s.push(call("write_hidden", "fs.write", json!({"mount_id":p,"path":"/.hidden","content":"h"})));
    s.push(call("list_root", "fs.list_dir", json!({"mount_id":p,"path":"/"})));
    s.push(call("list_root_hidden", "fs.list_dir", json!({"mount_id":p,"path":"/","include_hidden":true})));
    s.push(call("list_root_sizes", "fs.list_dir", json!({"mount_id":p,"path":"/","with_sizes":true})));
    s.push(call("tree_default", "fs.tree", json!({"mount_id":p})));
    s.push(call("tree_deep", "fs.tree", json!({"mount_id":p,"path":"/","max_depth":10,"with_sizes":true})));
    // the tolerant string array: array, bare string, csv, null
    s.push(call("tree_excl_array", "fs.tree", json!({"mount_id":p,"exclude_patterns":[".git","src"]})));
    s.push(call("tree_excl_string", "fs.tree", json!({"mount_id":p,"exclude_patterns":"src"})));
    s.push(call("tree_excl_csv", "fs.tree", json!({"mount_id":p,"exclude_patterns":".git, src"})));
    s.push(call("tree_excl_null", "fs.tree", json!({"mount_id":p,"exclude_patterns":Value::Null})));
    s.push(call("glob_all", "fs.glob", json!({"mount_id":p,"pattern":"**/*"})));
    s.push(call("glob_py", "fs.glob", json!({"mount_id":p,"pattern":"**/*.py"})));
    s.push(call("glob_excl_string", "fs.glob", json!({"mount_id":p,"pattern":"**/*","exclude_patterns":"*.py"})));
    s.push(call("grep_content", "fs.grep", json!({"mount_id":p,"pattern":"total"})));
    s.push(call("grep_files", "fs.grep", json!({"mount_id":p,"pattern":"total","output_mode":"files"})));
    s.push(call("grep_count", "fs.grep", json!({"mount_id":p,"pattern":"total","output_mode":"count"})));
    s.push(call("grep_literal", "fs.grep", json!({"mount_id":p,"pattern":"total","regex":false})));
    s.push(call("grep_icase", "fs.grep", json!({"mount_id":p,"pattern":"TOTAL","case_sensitive":false})));
    s.push(call("grep_context", "fs.grep", json!({"mount_id":p,"pattern":"total","context_lines":1})));
    s.push(call("read_many", "fs.read_many", json!({"mount_id":p,"paths":["/a.txt","/src/app.py","/does-not-exist"]})));
    s.push(call("read_section", "fs.read_section", json!({"mount_id":p,"path":"/src/app.py","anchor_line":2})));

    // ── symbols ──────────────────────────────────────────────────────────────
    s.push(call("find_def_py", "fs.find_definition", json!({"mount_id":p,"name":"hello"})));
    s.push(call("find_refs_py", "fs.find_references", json!({"mount_id":p,"name":"total"})));

    // ── copy / move / delete + refcount behaviour ────────────────────────────
    s.push(call("copy_file", "fs.copy", json!({"mount_id":p,"source":"/a.txt","destination":"/copy.txt"})));
    s.push(call("copy_noclobber", "fs.copy", json!({"mount_id":p,"source":"/a.txt","destination":"/copy.txt"})));
    s.push(call("copy_tree", "fs.copy", json!({"mount_id":p,"source":"/src","destination":"/src-copy","recursive":true})));
    s.push(call("move_file", "fs.move", json!({"mount_id":p,"source":"/copy.txt","destination":"/moved.txt"})));
    s.push(call("move_missing", "fs.move", json!({"mount_id":p,"source":"/nope","destination":"/x"})));
    s.push(call("stat_moved", "fs.stat", json!({"mount_id":p,"path":"/moved.txt"})));
    s.push(call("delete_moved", "fs.delete", json!({"mount_id":p,"path":"/moved.txt"})));
    s.push(call("exists_after_delete", "fs.exists", json!({"mount_id":p,"path":"/moved.txt"})));
    s.push(call("delete_missing", "fs.delete", json!({"mount_id":p,"path":"/nope.txt"})));
    s.push(call("delete_dir_nonrecursive", "fs.delete", json!({"mount_id":p,"path":"/src-copy"})));
    s.push(call("delete_dir_recursive", "fs.delete", json!({"mount_id":p,"path":"/src-copy","recursive":true})));

    // ── errors: path safety, missing files, wrong types ──────────────────────
    s.push(call("read_missing", "fs.read", json!({"mount_id":p,"path":"/no-such-file.txt"})));
    s.push(call("stat_missing", "fs.stat", json!({"mount_id":p,"path":"/no-such-file.txt"})));
    s.push(call("traversal", "fs.read", json!({"mount_id":p,"path":"/../../etc/passwd"})));
    s.push(call("read_a_dir", "fs.read", json!({"mount_id":p,"path":"/src"})));
    s.push(call("missing_required_arg", "fs.read", json!({"mount_id":p})));
    s.push(call("wrong_project", "fs.list_allowed_roots", json!({"mount_id":"no-such-project"})));
    s.push(call("list_allowed_roots", "fs.list_allowed_roots", json!({"mount_id":p})));

    // ── multi edit / search replace / patch ──────────────────────────────────
    s.push(call("write_multi", "fs.write", json!({"mount_id":p,"path":"/multi.txt","content":"one\ntwo\nthree\n"})));
    s.push(call("read_multi", "fs.read", json!({"mount_id":p,"path":"/multi.txt"})));
    s.push(call(
        "multi_edit_dry",
        "fs.multi_edit",
        json!({"mount_id":p,"path":"/multi.txt","dry_run":true,
               "edits":[{"old_string":"one","new_string":"1"},{"old_string":"two","new_string":"2"}]}),
    ));
    s.push(call(
        "multi_edit_apply",
        "fs.multi_edit",
        json!({"mount_id":p,"path":"/multi.txt",
               "edits":[{"old_string":"one","new_string":"1"},{"old_string":"two","new_string":"2"}]}),
    ));
    s.push(call("read_after_multi", "fs.read", json!({"mount_id":p,"path":"/multi.txt"})));
    s.push(call(
        "multi_edit_atomic_fail",
        "fs.multi_edit",
        json!({"mount_id":p,"path":"/multi.txt",
               "edits":[{"old_string":"three","new_string":"3"},{"old_string":"absent","new_string":"x"}]}),
    ));
    s.push(call("read_after_atomic_fail", "fs.read", json!({"mount_id":p,"path":"/multi.txt"})));
    s.push(call(
        "search_replace",
        "fs.search_replace",
        json!({"mount_id":p,"path":"/multi.txt","search_block":"three","replace_block":"3"}),
    ));

    // ── audit log (volatile timestamps are normalized away) ─────────────────
    s.push(call("audit_log", "fs.audit_log", json!({"mount_id":p})));
    s.push(call("audit_log_limit", "fs.audit_log", json!({"mount_id":p,"limit":3})));

    // ── unicode and boundaries ───────────────────────────────────────────────
    s.push(call("write_unicode", "fs.write", json!({"mount_id":p,"path":"/uni.txt","content":"héllo 🌍 日本語\n"})));
    s.push(call("read_unicode", "fs.read", json!({"mount_id":p,"path":"/uni.txt"})));
    s.push(call("hash_unicode", "fs.hash", json!({"mount_id":p,"path":"/uni.txt"})));
    s.push(call("write_special_name", "fs.write", json!({"mount_id":p,"path":"/a b&c'd.txt","content":"x"})));
    s.push(call("read_special_name", "fs.read", json!({"mount_id":p,"path":"/a b&c'd.txt"})));

    // ── REST data plane ──────────────────────────────────────────────────────
    s.push(rest_get("rest_roots", "/api/fs/roots"));
    s.push(rest_get("rest_list", &format!("/api/fs/{p}/list?path=/")));
    s.push(rest_get("rest_read", &format!("/api/fs/{p}/read?path=/a.txt")));
    s.push(rest_get("rest_stat", &format!("/api/fs/{p}/stat?path=/a.txt")));
    s.push(rest_get("rest_exists", &format!("/api/fs/{p}/exists?path=/a.txt")));
    s.push(rest_get("rest_hash", &format!("/api/fs/{p}/hash?path=/a.txt")));
    s.push(rest_get("rest_count_lines", &format!("/api/fs/{p}/count-lines?path=/a.txt")));
    s.push(rest_get("rest_tree", &format!("/api/fs/{p}/tree?path=/&max_depth=3")));
    s.push(rest_get("rest_glob", &format!("/api/fs/{p}/glob?pattern=**/*.py")));
    s.push(rest_get("rest_grep", &format!("/api/fs/{p}/grep?pattern=total")));
    s.push(rest_get("rest_head", &format!("/api/fs/{p}/head?path=/a.txt&lines=1")));
    s.push(rest_get("rest_tail", &format!("/api/fs/{p}/tail?path=/a.txt&lines=1")));
    s.push(rest_get("rest_read_bytes", &format!("/api/fs/{p}/read-bytes?path=/a.txt&offset_bytes=0&length_bytes=4")));
    s.push(rest_get("rest_audit", &format!("/api/fs/{p}/audit-log")));
    s.push(rest_post("rest_write", &format!("/api/fs/{p}/write"), json!({"path":"/rest.txt","content":"via rest"})));
    s.push(rest_get("rest_read_written", &format!("/api/fs/{p}/read?path=/rest.txt")));
    s.push(rest_post("rest_mkdir", &format!("/api/fs/{p}/mkdir"), json!({"path":"/rest-dir"})));
    s.push(rest_post("rest_copy", &format!("/api/fs/{p}/copy"), json!({"source":"/rest.txt","destination":"/rest2.txt"})));
    s.push(rest_post("rest_delete", &format!("/api/fs/{p}/delete"), json!({"path":"/rest2.txt"})));
    s.push(rest_get("rest_missing", &format!("/api/fs/{p}/read?path=/nope.txt")));
    s.push(rest_get("rest_forbidden_project", "/api/fs/no-such-project/list?path=/"));

    // ── OpenAPI surface (public) ─────────────────────────────────────────────
    s.push(Step::Public { label: "swagger_json".into(), path: "/api/swagger.json".into() });

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_substantial_and_labels_are_unique() {
        let c = build("parity");
        assert!(c.len() > 100, "corpus too small: {}", c.len());
        let mut labels: Vec<&str> = c.iter().map(|s| s.label()).collect();
        labels.sort_unstable();
        let before = labels.len();
        labels.dedup();
        assert_eq!(before, labels.len(), "duplicate step labels in the corpus");
    }

    #[test]
    fn corpus_covers_every_surface() {
        let c = build("parity");
        let has = |needle: &str| c.iter().any(|s| s.label().contains(needle));
        assert!(has("tools_list"));
        assert!(has("initialize"));
        assert!(has("unknown_tool"));
        assert!(has("admin_"));
        assert!(has("rest_"));
        assert!(has("swagger"));
        assert!(has("traversal"));
        assert!(has("audit"));
    }

    #[test]
    fn tolerant_array_shapes_are_all_exercised() {
        let c = build("parity");
        for l in ["tree_excl_array", "tree_excl_string", "tree_excl_csv", "tree_excl_null"] {
            assert!(c.iter().any(|s| s.label() == l), "missing step {l}");
        }
    }
}
