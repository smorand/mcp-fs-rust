//! Declarative tool definition: builds the JSON Schema for a tool's arguments
//! byte-for-byte compatible with what the C# MCP SDK generates from
//! `[Description]` attributes, and provides typed argument accessors.
//!
//! The C# schema shape, verified against the running server:
//! ```json
//! {"type":"object",
//!  "properties":{"mount_id":{"description":"...","type":"string"},
//!                "algo":{"description":"...","type":"string","default":"sha256"}},
//!  "required":["mount_id","path"]}
//! ```
//! Notes reproduced exactly:
//! * `description` comes FIRST, then `type`, then `default` (property key order).
//! * `required` is omitted entirely when there are no required params.
//! * A nullable optional renders `"default": null`.
//! * `since` (nullable number) renders `"type": ["number","null"]`.
//! * A tolerant string-array param (`exclude_patterns`) renders with NO `type`.

use serde_json::{Map, Value, json};

/// Parameter kind, controlling the generated `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Str,
    Int,
    Num,
    Bool,
    /// Array of strings with an explicit `items` schema.
    StrArray,
    /// Nullable number: renders `"type": ["number","null"]`.
    NullableNum,
    /// Tolerant string array: renders with NO `type` at all (matches the C#
    /// `FlexibleStringArrayConverter`, which accepts array | string | csv | null).
    FlexibleStrArray,
    /// Free-form object array (e.g. `edits`), with a caller-supplied items schema.
    ObjArray(&'static str),
}

#[derive(Debug, Clone)]
struct Param {
    name: &'static str,
    desc: &'static str,
    ty: ParamType,
    required: bool,
    /// Default rendered into the schema. `None` = omit the `default` key.
    default: Option<Value>,
}

/// Builder producing a tool's name, description and argument JSON Schema.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: &'static str,
    pub description: String,
    params: Vec<Param>,
}

impl ToolSchema {
    pub fn new(name: &'static str, description: impl Into<String>) -> Self {
        Self { name, description: description.into(), params: Vec::new() }
    }

    fn push(
        mut self,
        name: &'static str,
        desc: &'static str,
        ty: ParamType,
        required: bool,
        default: Option<Value>,
    ) -> Self {
        self.params.push(Param { name, desc, ty, required, default });
        self
    }

    // ── required params ──────────────────────────────────────────────────────
    pub fn req_str(self, n: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::Str, true, None)
    }
    pub fn req_int(self, n: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::Int, true, None)
    }
    pub fn req_str_array(self, n: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::StrArray, true, None)
    }
    pub fn req_obj_array(self, n: &'static str, d: &'static str, items: &'static str) -> Self {
        self.push(n, d, ParamType::ObjArray(items), true, None)
    }

    // ── optional params with a default rendered in the schema ────────────────
    pub fn opt_str(self, n: &'static str, def: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::Str, false, Some(json!(def)))
    }
    /// Optional string with no value: renders `"default": null`.
    pub fn opt_str_null(self, n: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::Str, false, Some(Value::Null))
    }
    pub fn opt_int(self, n: &'static str, def: i64, d: &'static str) -> Self {
        self.push(n, d, ParamType::Int, false, Some(json!(def)))
    }
    pub fn opt_bool(self, n: &'static str, def: bool, d: &'static str) -> Self {
        self.push(n, d, ParamType::Bool, false, Some(json!(def)))
    }
    /// Nullable number (`since`): `"type": ["number","null"], "default": null`.
    pub fn opt_nullable_num(self, n: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::NullableNum, false, Some(Value::Null))
    }
    /// Tolerant string array (`exclude_patterns`): no `type`, `"default": null`.
    pub fn opt_flexible_str_array(self, n: &'static str, d: &'static str) -> Self {
        self.push(n, d, ParamType::FlexibleStrArray, false, Some(Value::Null))
    }

    /// Render the `inputSchema` object.
    pub fn input_schema(&self) -> Value {
        let mut props = Map::new();
        for p in &self.params {
            let mut o = Map::new();
            // key order: description, type, default  (matches the C# generator)
            o.insert("description".into(), json!(p.desc));
            match p.ty {
                ParamType::Str => {
                    o.insert("type".into(), json!("string"));
                }
                ParamType::Int => {
                    o.insert("type".into(), json!("integer"));
                }
                ParamType::Num => {
                    o.insert("type".into(), json!("number"));
                }
                ParamType::Bool => {
                    o.insert("type".into(), json!("boolean"));
                }
                ParamType::StrArray => {
                    o.insert("type".into(), json!("array"));
                    o.insert("items".into(), json!({"type": "string"}));
                }
                ParamType::NullableNum => {
                    o.insert("type".into(), json!(["number", "null"]));
                }
                ParamType::FlexibleStrArray => { /* no type, by design */ }
                ParamType::ObjArray(items) => {
                    o.insert("type".into(), json!("array"));
                    let items: Value = serde_json::from_str(items)
                        .expect("tool obj-array items schema must be valid JSON");
                    o.insert("items".into(), items);
                }
            }
            if let Some(d) = &p.default {
                o.insert("default".into(), d.clone());
            }
            props.insert(p.name.to_string(), Value::Object(o));
        }

        let required: Vec<&str> =
            self.params.iter().filter(|p| p.required).map(|p| p.name).collect();

        let mut schema = Map::new();
        schema.insert("type".into(), json!("object"));
        schema.insert("properties".into(), Value::Object(props));
        // The C# generator omits `required` entirely when empty.
        if !required.is_empty() {
            schema.insert("required".into(), json!(required));
        }
        Value::Object(schema)
    }

    /// The `tools/list` entry for this tool.
    pub fn to_list_entry(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact schema captured from the running C# server for fs.hash.
    #[test]
    fn fs_hash_schema_matches_csharp() {
        let s = ToolSchema::new("fs.hash", "Content hash (md5|sha1|sha256|sha512).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .opt_str("algo", "sha256", "Hash algorithm: md5, sha1, sha256, or sha512.");

        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "algo":{"description":"Hash algorithm: md5, sha1, sha256, or sha512.","type":"string","default":"sha256"}},
               "required":["mount_id","path"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    /// No-param tools render `{"type":"object","properties":{}}` with no `required`.
    #[test]
    fn no_param_tool_omits_required() {
        let s = ToolSchema::new("admin.list_projects", "List projects the caller can access.");
        let expected: Value =
            serde_json::from_str(r#"{"type":"object","properties":{}}"#).unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    /// `since` is a nullable number in the C# schema.
    #[test]
    fn nullable_number_renders_type_array() {
        let s = ToolSchema::new("fs.audit_log", "Recent mutations performed in this session.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .opt_nullable_num("since", "Only return entries at or after this Unix timestamp (seconds).")
            .opt_int("limit", 20, "Maximum number of recent entries to return.");
        let v = s.input_schema();
        assert_eq!(v["properties"]["since"]["type"], json!(["number", "null"]));
        assert_eq!(v["properties"]["since"]["default"], Value::Null);
        assert_eq!(v["properties"]["limit"]["default"], json!(20));
    }

    /// The tolerant string array carries no `type` (C# FlexibleStringArrayConverter).
    #[test]
    fn flexible_string_array_has_no_type() {
        let s = ToolSchema::new("fs.glob", "Find files by glob pattern, newest first (cap 100).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("pattern", "Glob pattern to match file paths, e.g. **/*.cs.")
            .opt_str("root", "/", "Absolute POSIX directory to search under.")
            .opt_flexible_str_array(
                "exclude_patterns",
                "Glob patterns whose matches are excluded from results.",
            );
        let v = s.input_schema();
        let ep = &v["properties"]["exclude_patterns"];
        assert!(ep.get("type").is_none(), "exclude_patterns must have no type");
        assert_eq!(ep["default"], Value::Null);
        assert_eq!(v["required"], json!(["mount_id", "pattern"]));
    }

    #[test]
    fn str_array_has_items() {
        let s = ToolSchema::new("fs.read_many", "Batch read several files with per-file error isolation.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str_array("paths", "Absolute POSIX paths to read, one entry per file.")
            .opt_int("per_file_cap_lines", 500, "Maximum number of lines returned per file.");
        let v = s.input_schema();
        assert_eq!(v["properties"]["paths"]["type"], json!("array"));
        assert_eq!(v["properties"]["paths"]["items"], json!({"type":"string"}));
    }
}
