//! Tool catalog entries: a transport-agnostic description of one tool, for
//! consumers (e.g. the REST API docs catalog, US-0013) that need the schema
//! and behaviour annotations without going through `mcp::ToolRegistry`.

use crate::tools::registry_support::ToolSchema;
use serde_json::Value;

/// One tool's name, description, JSON Schema and behaviour hints.
#[derive(Debug, Clone)]
pub struct ToolCatalogEntry {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
    pub idempotent: bool,
    pub open_world: bool,
    pub destructive: bool,
}

/// Build a [`ToolCatalogEntry`] from a `tools/list` entry (`ToolSchema::to_list_entry`).
fn entry_from_list_entry(v: &Value) -> ToolCatalogEntry {
    let ann = v.get("annotations").cloned().unwrap_or_default();
    ToolCatalogEntry {
        name: v["name"].as_str().unwrap_or_default().to_string(),
        description: v["description"].as_str().unwrap_or_default().to_string(),
        input_schema: v["inputSchema"].clone(),
        read_only: ann.get("readOnlyHint").and_then(Value::as_bool).unwrap_or(false),
        idempotent: ann.get("idempotentHint").and_then(Value::as_bool).unwrap_or(false),
        open_world: ann.get("openWorldHint").and_then(Value::as_bool).unwrap_or(false),
        destructive: ann.get("destructiveHint").and_then(Value::as_bool).unwrap_or(false),
    }
}

impl From<&ToolSchema> for ToolCatalogEntry {
    fn from(schema: &ToolSchema) -> Self {
        entry_from_list_entry(&schema.to_list_entry())
    }
}
