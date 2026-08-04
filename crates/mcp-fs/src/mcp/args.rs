//! Typed accessors over a tool call's `arguments` object.
//!
//! Two behaviours are reproduced from the C# implementation:
//! * A missing required argument is `ERR_INVALID_ARGUMENT`.
//! * String arrays are **tolerant** (`FlexibleStringArrayConverter`): an array, a
//!   bare string, a comma separated string, an empty string, or null are all
//!   accepted and normalized to `Vec<String>`.

use crate::errors::{Result, ToolError};
use serde_json::Value;

/// Wrapper around the `arguments` object of a `tools/call`.
#[derive(Debug, Clone)]
pub struct Args(pub Value);

impl Args {
    pub fn new(v: Value) -> Self {
        // A missing/None `arguments` is treated as an empty object.
        match v {
            Value::Object(_) => Self(v),
            _ => Self(Value::Object(Default::default())),
        }
    }

    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|v| !v.is_null())
    }

    fn missing(name: &str) -> ToolError {
        ToolError::invalid_argument(format!("missing required argument '{name}'"))
    }

    fn wrong(name: &str, want: &str) -> ToolError {
        ToolError::invalid_argument(format!("argument '{name}' must be {want}"))
    }

    // ── required ─────────────────────────────────────────────────────────────
    pub fn str(&self, name: &str) -> Result<String> {
        match self.get(name) {
            None => Err(Self::missing(name)),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(Self::wrong(name, "a string")),
        }
    }

    pub fn int(&self, name: &str) -> Result<i64> {
        match self.get(name) {
            None => Err(Self::missing(name)),
            Some(v) => v
                .as_i64()
                .or_else(|| v.as_f64().map(|f| f as i64))
                .ok_or_else(|| Self::wrong(name, "an integer")),
        }
    }

    // ── optional with default ────────────────────────────────────────────────
    pub fn str_or(&self, name: &str, default: &str) -> String {
        match self.get(name) {
            Some(Value::String(s)) => s.clone(),
            _ => default.to_string(),
        }
    }

    pub fn opt_str(&self, name: &str) -> Option<String> {
        match self.get(name) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn int_or(&self, name: &str, default: i64) -> i64 {
        self.get(name)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(default)
    }

    pub fn bool_or(&self, name: &str, default: bool) -> bool {
        match self.get(name) {
            Some(Value::Bool(b)) => *b,
            // tolerate "true"/"false" strings, as a lenient LLM might send them
            Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                "true" => true,
                "false" => false,
                _ => default,
            },
            _ => default,
        }
    }

    pub fn opt_num(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(|v| v.as_f64())
    }

    /// Tolerant string array. Accepts:
    /// array of scalars, a bare string, a comma separated string, an empty
    /// string (=> empty vec), or absent/null (=> empty vec).
    pub fn str_array(&self, name: &str) -> Vec<String> {
        match self.get(name) {
            None => Vec::new(),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect(),
            Some(Value::String(s)) => {
                if s.is_empty() {
                    Vec::new()
                } else {
                    s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
                }
            }
            Some(_) => Vec::new(),
        }
    }

    /// Required string array (e.g. `paths`), tolerant on shape but must be present.
    pub fn req_str_array(&self, name: &str) -> Result<Vec<String>> {
        if self.get(name).is_none() {
            return Err(Self::missing(name));
        }
        Ok(self.str_array(name))
    }

    /// Raw access for structured params (e.g. `edits`).
    pub fn raw(&self, name: &str) -> Option<&Value> {
        self.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn a(v: Value) -> Args { Args::new(v) }

    #[test]
    fn required_string_present_and_missing() {
        let x = a(json!({"mount_id": "p"}));
        assert_eq!(x.str("mount_id").unwrap(), "p");
        let e = x.str("path").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("missing required argument 'path'"));
    }

    #[test]
    fn null_counts_as_absent() {
        let x = a(json!({"branch": null}));
        assert_eq!(x.opt_str("branch"), None);
        assert_eq!(x.str_or("branch", "main"), "main");
    }

    #[test]
    fn defaults_are_applied() {
        let x = a(json!({}));
        assert_eq!(x.int_or("limit", 20), 20);
        assert!(x.bool_or("trash", true));
        assert_eq!(x.str_or("root", "/"), "/");
    }

    #[test]
    fn ints_accept_floats() {
        let x = a(json!({"limit": 5.0}));
        assert_eq!(x.int_or("limit", 20), 5);
    }

    /// The four shapes the C# FlexibleStringArrayConverter accepts.
    #[test]
    fn flexible_string_array_all_shapes() {
        assert_eq!(a(json!({"e": ["x","y"]})).str_array("e"), vec!["x", "y"]);
        assert_eq!(a(json!({"e": ".git"})).str_array("e"), vec![".git"]);
        assert_eq!(
            a(json!({"e": ".git, node_modules"})).str_array("e"),
            vec![".git", "node_modules"]
        );
        assert!(a(json!({"e": ""})).str_array("e").is_empty());
        assert!(a(json!({"e": null})).str_array("e").is_empty());
        assert!(a(json!({})).str_array("e").is_empty());
    }

    #[test]
    fn array_coerces_scalars_to_strings() {
        assert_eq!(a(json!({"e": [1, true, "z"]})).str_array("e"), vec!["1", "true", "z"]);
    }

    #[test]
    fn required_array_missing_errors() {
        let e = a(json!({})).req_str_array("paths").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[test]
    fn non_object_arguments_become_empty() {
        let x = Args::new(Value::Null);
        assert_eq!(x.int_or("limit", 7), 7);
    }
}
