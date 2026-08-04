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
    s.to_string()
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
    fn relax_messages_blanks_free_form_text() {
        let v = json!({"detail":"not a file: /x"});
        let n = normalize(&v, &Options { relax_messages: true, ..Default::default() });
        assert_eq!(n["detail"], "<message>");
    }
}
