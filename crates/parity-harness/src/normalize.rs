//! Normalization applied before diffing two servers' responses.
//!
//! Some values legitimately differ between two runs or two implementations
//! without breaking the contract: wall-clock timestamps, the server version,
//! generated ids, absolute host paths, and JSON key order inside an object.
//! We replace those with stable placeholders so a diff shows only real
//! behavioural differences.

use serde_json::{Map, Value};

/// Keys whose value is inherently volatile (time, identity of a run, version).
const VOLATILE_KEYS: &[&str] = &[
    "mtime",
    "ctime",
    "atime",
    "timestamp",
    "created_at",
    "added_at",
    "expires_at",
    "expires_in",
    "version",
    "date",
    "duration_ms",
    "elapsed_ms",
];

/// Keys whose value is a free-form human message that may word things differently
/// for the same `ERR_*` code. The code itself is still compared (see `extract_err_code`).
const MESSAGE_KEYS: &[&str] = &["detail", "message"];

pub struct Options {
    /// Compare only the `ERR_*` code of an error text, not the whole sentence.
    pub codes_only_for_errors: bool,
    /// Blank out free-form message fields.
    pub relax_messages: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { codes_only_for_errors: true, relax_messages: false }
    }
}

/// Pull the `ERR_*` token out of a message, if there is one.
pub fn extract_err_code(text: &str) -> Option<&str> {
    let start = text.find("ERR_")?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_uppercase() || c == '_'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Recursively normalize a response value.
pub fn normalize(v: &Value, opts: &Options) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, val) in m {
                if VOLATILE_KEYS.contains(&k.as_str()) {
                    out.insert(k.clone(), Value::String("<volatile>".into()));
                    continue;
                }
                if opts.relax_messages && MESSAGE_KEYS.contains(&k.as_str()) {
                    out.insert(k.clone(), Value::String("<message>".into()));
                    continue;
                }
                out.insert(k.clone(), normalize(val, opts));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| normalize(x, opts)).collect()),
        Value::String(s) => Value::String(normalize_string(s, opts)),
        other => other.clone(),
    }
}

/// Mask the epoch milliseconds a trash path embeds: `/.mcp_trash/1785863086054__x`.
/// The stamp is as volatile as an `mtime`, it is just carried inside a string.
fn mask_trash_stamp(s: &str) -> Option<String> {
    let marker = "/.mcp_trash/";
    let at = s.find(marker)?;
    let rest = &s[at + marker.len()..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    Some(format!(
        "{}{}<stamp>{}",
        &s[..at],
        marker,
        &rest[digits..]
    ))
}

/// A trash entry surfaces as a bare `1785863086054__moved.txt` file name in a listing,
/// with no directory prefix to key on. The leading epoch is as volatile as the full path.
fn mask_bare_trash_name(s: &str) -> Option<String> {
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits >= 10 && s[digits..].starts_with("__") {
        return Some(format!("<stamp>{}", &s[digits..]));
    }
    None
}

fn normalize_string(s: &str, opts: &Options) -> String {
    // A tool error text is "An error occurred invoking '<tool>': ERR_X: <sentence>".
    // Keep the tool and the code, drop the sentence, so wording differences in a
    // human message do not fail parity while a wrong code still does.
    if opts.codes_only_for_errors
        && let Some(code) = extract_err_code(s)
    {
        if let Some(tool_start) = s.find('\'')
            && let Some(tool_end) = s[tool_start + 1..].find('\'')
        {
            let tool = &s[tool_start + 1..tool_start + 1 + tool_end];
            return format!("<error tool={tool} code={code}>");
        }
        return format!("<error code={code}>");
    }
    // Absolute host paths differ between two checkouts.
    if s.contains("/Users/") || s.contains("/private/var/") || s.contains("/tmp/") {
        return "<host-path>".into();
    }
    if let Some(masked) = mask_trash_stamp(s) {
        return masked;
    }
    if let Some(masked) = mask_bare_trash_name(s) {
        return masked;
    }
    s.to_string()
}

/// Post-process a normalized response for the few steps whose payload depends on
/// the host environment or on an unspecified ordering, rather than on behaviour.
///
/// This is deliberately narrow. Each case is something the contract does not pin
/// down, so comparing it would report noise instead of a real difference:
/// * `tools/list` order: MCP clients look tools up by name, so only the SET and each
///   tool's schema matter. Sorting by name keeps that check strict while dropping the
///   registration order, which is an implementation detail on both sides.
/// * `admin.list_all_projects` / `admin.list_users`: the reference server carries other
///   projects from real use. Only the probe project is comparable, so the lists are
///   filtered down to it.
pub fn tame_environment(label: &str, v: &mut Value, project: &str) {
    match label {
        "tools_list" => {
            if let Some(tools) = v
                .get_mut("result")
                .and_then(|r| r.get_mut("tools"))
                .and_then(|t| t.as_array_mut())
            {
                tools.sort_by(|a, b| {
                    a.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .cmp(b.get("name").and_then(Value::as_str).unwrap_or_default())
                });
            }
        }
        "admin_list_projects" | "admin_list_all_projects" => {
            retain_in_tool_payload(v, "projects", |e| {
                e.get("project_id").and_then(Value::as_str) == Some(project)
            });
        }
        "swagger_json" => normalize_component_refs(v),
        // The caller may own volumes from earlier runs on either server.
        "list_allowed_roots" => {
            retain_in_tool_payload(v, "roots", |e| {
                e.get("mount_id").and_then(Value::as_str) == Some("<project>")
                    || e.get("mount_id").and_then(Value::as_str) == Some(project)
            });
        }
        "rest_roots" => {
            if let Some(arr) = v.get_mut("roots").and_then(|a| a.as_array_mut()) {
                arr.retain(|e| {
                    let m = e.get("mount_id").and_then(Value::as_str);
                    m == Some("<project>") || m == Some(project)
                });
            }
        }
        "admin_list_users" => {
            // Only the probe owner is guaranteed to exist on both servers.
            retain_in_tool_payload(v, "users", |e| {
                e.get("is_admin").and_then(Value::as_bool) == Some(true)
            });
        }
        _ => {}
    }
}

/// Replace every occurrence of the per run project id with a placeholder, in keys'
/// values and inside strings (paths, messages, mount ids). Without this, a capture and a
/// compare using different fresh ids would differ everywhere.
pub fn mask_project(v: &mut Value, project: &str) {
    match v {
        Value::String(s) => {
            if s.contains(project) {
                *s = s.replace(project, "<project>");
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                mask_project(x, project);
            }
        }
        Value::Object(m) => {
            for (_, x) in m.iter_mut() {
                mask_project(x, project);
            }
        }
        _ => {}
    }
}

/// A component schema name is not part of the API contract (the schema shape is), and
/// the reference generator appends a disambiguating digit on a name collision
/// (`MkdirBody2`). Strip a trailing digit run from a `$ref` so only the shape is compared.
pub fn normalize_component_refs(v: &mut Value) {
    match v {
        Value::Object(m) => {
            let renamed: Vec<(String, Value)> = m
                .iter()
                .map(|(k, val)| (k.clone(), val.clone()))
                .collect();
            for (k, mut val) in renamed {
                if k == "$ref"
                    && let Some(r) = val.as_str()
                {
                    let stripped = r.trim_end_matches(|c: char| c.is_ascii_digit());
                    m.insert(k.clone(), Value::String(stripped.to_string()));
                    continue;
                }
                normalize_component_refs(&mut val);
                m.insert(k, val);
            }
            // Component definitions carry the same disambiguated names as keys.
            if let Some(schemas) = m.get_mut("schemas").and_then(|s| s.as_object_mut()) {
                let keys: Vec<String> = schemas.keys().cloned().collect();
                for k in keys {
                    let stripped = k.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
                    if stripped != k
                        && let Some(val) = schemas.remove(&k)
                    {
                        schemas.insert(stripped, val);
                    }
                }
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                normalize_component_refs(x);
            }
        }
        _ => {}
    }
}

/// Filter an array inside a decoded tool payload (`result.content[0].text.<key>`).
fn retain_in_tool_payload(v: &mut Value, key: &str, keep: impl Fn(&Value) -> bool) {
    if let Some(arr) = v
        .get_mut("result")
        .and_then(|r| r.get_mut("content"))
        .and_then(|c| c.get_mut(0))
        .and_then(|b| b.get_mut("text"))
        .and_then(|t| t.get_mut(key))
        .and_then(|a| a.as_array_mut())
    {
        arr.retain(&keep);
    }
}

/// The tool result payload is JSON encoded inside a text content block. Decode it
/// so we diff structure rather than a string, then normalize.
pub fn normalize_tool_text(text: &str, opts: &Options) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(inner) => normalize(&inner, opts),
        Err(_) => Value::String(normalize_string(text, opts)),
    }
}

/// Normalize a full JSON-RPC response, decoding nested tool payloads.
pub fn normalize_rpc(v: &Value, opts: &Options) -> Value {
    let mut out = v.clone();
    if let Some(content) = out
        .get_mut("result")
        .and_then(|r| r.get_mut("content"))
        .and_then(|c| c.as_array_mut())
    {
        for block in content.iter_mut() {
            if let Some(t) = block.get("text").and_then(Value::as_str) {
                let decoded = normalize_tool_text(t, opts);
                if let Some(obj) = block.as_object_mut() {
                    obj.insert("text".into(), decoded);
                }
            }
        }
    }
    normalize(&out, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn volatile_keys_are_masked() {
        let v = json!({"path":"/a.txt","mtime":1785690473.263,"size":5});
        let n = normalize(&v, &Options::default());
        assert_eq!(n["mtime"], "<volatile>");
        assert_eq!(n["size"], 5, "non volatile values survive");
        assert_eq!(n["path"], "/a.txt");
    }

    #[test]
    fn nested_volatile_keys_are_masked() {
        let v = json!({"entries":[{"name":"a","mtime":1.0},{"name":"b","mtime":2.0}]});
        let n = normalize(&v, &Options::default());
        assert_eq!(n["entries"][0]["mtime"], "<volatile>");
        assert_eq!(n["entries"][1]["mtime"], "<volatile>");
        assert_eq!(n["entries"][1]["name"], "b");
    }

    #[test]
    fn err_code_is_extracted_from_a_message() {
        assert_eq!(extract_err_code("ERR_FORBIDDEN: nope"), Some("ERR_FORBIDDEN"));
        assert_eq!(
            extract_err_code("An error occurred invoking 'fs.read': ERR_NOT_FOUND: '/x' not found"),
            Some("ERR_NOT_FOUND")
        );
        assert_eq!(extract_err_code("all good"), None);
    }

    #[test]
    fn error_text_collapses_to_tool_and_code() {
        let s = "An error occurred invoking 'fs.list_allowed_roots': ERR_FORBIDDEN: 'a@b.c' is not a member of 'p'";
        let n = normalize_string(s, &Options::default());
        assert_eq!(n, "<error tool=fs.list_allowed_roots code=ERR_FORBIDDEN>");
    }

    #[test]
    fn two_wordings_of_the_same_code_compare_equal() {
        let a = "An error occurred invoking 'fs.read': ERR_NOT_FOUND: '/x.txt' not found";
        let b = "An error occurred invoking 'fs.read': ERR_NOT_FOUND: no such file '/x.txt'";
        let o = Options::default();
        assert_eq!(normalize_string(a, &o), normalize_string(b, &o));
    }

    #[test]
    fn a_different_code_still_differs() {
        let a = "An error occurred invoking 'fs.read': ERR_NOT_FOUND: x";
        let b = "An error occurred invoking 'fs.read': ERR_FORBIDDEN: x";
        let o = Options::default();
        assert_ne!(normalize_string(a, &o), normalize_string(b, &o));
    }

    #[test]
    fn nested_tool_payload_is_decoded_then_normalized() {
        let rpc = json!({
            "result": {"content": [{"type":"text","text":"{\"path\":\"/a.txt\",\"mtime\":123.4}"}]},
            "id": 1, "jsonrpc": "2.0"
        });
        let n = normalize_rpc(&rpc, &Options::default());
        let inner = &n["result"]["content"][0]["text"];
        assert_eq!(inner["path"], "/a.txt");
        assert_eq!(inner["mtime"], "<volatile>");
    }

    #[test]
    fn host_paths_are_masked() {
        let v = json!({"cfg":"/Users/someone/projects/x/config.yaml"});
        let n = normalize(&v, &Options::default());
        assert_eq!(n["cfg"], "<host-path>");
    }

    #[test]
    fn key_order_does_not_matter_after_normalization() {
        // serde_json with preserve_order keeps insertion order, so compare parsed
        // values rather than raw strings; equality is structural.
        let a: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        let o = Options::default();
        assert_eq!(normalize(&a, &o), normalize(&b, &o));
    }

    #[test]
    fn trash_stamp_is_masked() {
        let a = normalize_string("/.mcp_trash/1785863086054__moved.txt", &Options::default());
        let b = normalize_string("/.mcp_trash/1785867506241__moved.txt", &Options::default());
        assert_eq!(a, b, "two runs must compare equal");
        assert_eq!(a, "/.mcp_trash/<stamp>__moved.txt");
    }

    #[test]
    fn trash_stamp_masking_survives_a_prefix() {
        let s = normalize_string("-> /.mcp_trash/1785863086054__x", &Options::default());
        assert_eq!(s, "-> /.mcp_trash/<stamp>__x");
    }

    #[test]
    fn a_non_trash_path_is_untouched() {
        assert_eq!(normalize_string("/a/b.txt", &Options::default()), "/a/b.txt");
    }

    #[test]
    fn tools_list_is_compared_order_independently() {
        let mk = |names: [&str; 2]| {
            json!({"result": {"tools": [
                {"name": names[0], "inputSchema": {"type":"object"}},
                {"name": names[1], "inputSchema": {"type":"object"}}
            ]}})
        };
        let mut a = mk(["fs.read_many", "fs.read_bytes"]);
        let mut b = mk(["fs.read_bytes", "fs.read_many"]);
        tame_environment("tools_list", &mut a, "p");
        tame_environment("tools_list", &mut b, "p");
        assert_eq!(a, b, "registration order is not part of the contract");
    }

    #[test]
    fn a_missing_tool_still_fails_after_sorting() {
        let mut a = json!({"result": {"tools": [{"name": "fs.read"}, {"name": "fs.write"}]}});
        let mut b = json!({"result": {"tools": [{"name": "fs.read"}]}});
        tame_environment("tools_list", &mut a, "p");
        tame_environment("tools_list", &mut b, "p");
        assert_ne!(a, b, "a missing tool must still be reported");
    }

    #[test]
    fn foreign_projects_are_filtered_out() {
        let mut v = json!({"result": {"content": [{"type":"text","text": {"projects": [
            {"project_id": "other-thing"},
            {"project_id": "probe"}
        ]}}]}});
        tame_environment("admin_list_all_projects", &mut v, "probe");
        let arr = v["result"]["content"][0]["text"]["projects"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["project_id"], "probe");
    }

    #[test]
    fn taming_leaves_other_labels_alone() {
        let before = json!({"result": {"content": [{"type":"text","text": {"projects": [{"project_id":"x"}]}}]}});
        let mut after = before.clone();
        tame_environment("some_other_step", &mut after, "probe");
        assert_eq!(before, after);
    }

    #[test]
    fn bare_trash_file_name_is_masked() {
        let a = normalize_string("1785863086054__moved.txt", &Options::default());
        let b = normalize_string("1785867506241__moved.txt", &Options::default());
        assert_eq!(a, b);
        assert_eq!(a, "<stamp>__moved.txt");
    }

    #[test]
    fn a_normal_numeric_name_is_not_masked() {
        // too short to be an epoch, and no double underscore
        assert_eq!(normalize_string("2024_report.txt", &Options::default()), "2024_report.txt");
        assert_eq!(normalize_string("12__x", &Options::default()), "12__x");
    }

    #[test]
    fn own_project_list_is_filtered_too() {
        let mut v = json!({"result": {"content": [{"type":"text","text": {"projects": [
            {"project_id": "leftover"}, {"project_id": "probe"}
        ]}}]}});
        tame_environment("admin_list_projects", &mut v, "probe");
        let arr = v["result"]["content"][0]["text"]["projects"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn roots_are_filtered_to_the_probe_on_both_shapes() {
        let mut mcp = json!({"result": {"content": [{"type":"text","text": {"roots": [
            {"mount_id": "leftover"}, {"mount_id": "probe"}
        ]}}]}});
        tame_environment("list_allowed_roots", &mut mcp, "probe");
        assert_eq!(mcp["result"]["content"][0]["text"]["roots"].as_array().unwrap().len(), 1);

        let mut rest = json!({"roots": [{"mount_id": "leftover"}, {"mount_id": "probe"}]});
        tame_environment("rest_roots", &mut rest, "probe");
        assert_eq!(rest["roots"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn relax_messages_blanks_free_form_text() {
        let v = json!({"detail":"not a file: /x"});
        let n = normalize(&v, &Options { relax_messages: true, ..Default::default() });
        assert_eq!(n["detail"], "<message>");
    }
}
