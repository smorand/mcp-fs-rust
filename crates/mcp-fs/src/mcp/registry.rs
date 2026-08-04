//! Tool registry: name -> (schema, handler).
//!
//! Every tool family registers its tools here. `tools/list` renders the schemas
//! in registration order; `tools/call` dispatches by exact name, with the same
//! dot/underscore tolerance the C# agent-side used to need (a client sending
//! `admin_list_projects` still reaches `admin.list_projects`).

use crate::errors::{Result, ToolError};
use crate::mcp::{Args, ToolSchema};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Per-request context handed to every tool handler.
///
/// `person` is the authenticated identity (email claim). `state` carries the
/// shared server state (stores, safety, config). Both are cheap to clone.
#[derive(Clone)]
pub struct ToolCtx {
    pub person: String,
    pub state: Arc<crate::state::AppState>,
}

/// Handlers own their inputs, so the returned future is `'static`.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A tool handler: takes the caller context plus parsed arguments, returns the
/// tool's result object (which is serialized as JSON text in the MCP response).
pub type ToolHandler = Arc<dyn Fn(ToolCtx, Args) -> BoxFuture<Result<Value>> + Send + Sync>;

pub struct RegisteredTool {
    pub schema: ToolSchema,
    pub handler: ToolHandler,
}

#[derive(Default)]
pub struct ToolRegistry {
    order: Vec<&'static str>,
    tools: HashMap<&'static str, RegisteredTool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one tool. Panics on a duplicate name: that is a build-time bug.
    pub fn add(&mut self, schema: ToolSchema, handler: ToolHandler) {
        let name = schema.name;
        assert!(
            !self.tools.contains_key(name),
            "duplicate tool registration: {name}"
        );
        self.order.push(name);
        self.tools.insert(name, RegisteredTool { schema, handler });
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn names(&self) -> &[&'static str] {
        &self.order
    }

    /// Resolve a tool name, tolerating dot/underscore confusion from clients.
    pub fn resolve(&self, name: &str) -> Option<&RegisteredTool> {
        if let Some(t) = self.tools.get(name) {
            return Some(t);
        }
        let normalized = name.replace('.', "_");
        self.order
            .iter()
            .find(|n| n.replace('.', "_") == normalized)
            .and_then(|n| self.tools.get(n))
    }

    /// The `tools/list` payload, in registration order.
    pub fn list_payload(&self) -> Value {
        let tools: Vec<Value> = self
            .order
            .iter()
            .map(|n| self.tools[n].schema.to_list_entry())
            .collect();
        serde_json::json!({"tools": tools})
    }

    /// Dispatch a call. `None` means the tool does not exist (JSON-RPC -32602).
    pub async fn call(&self, name: &str, ctx: ToolCtx, args: Args) -> Option<Result<Value>> {
        let tool = self.resolve(name)?;
        Some((tool.handler)(ctx, args).await)
    }
}

/// Helper to build a handler from an async closure without ceremony at call sites.
///
/// ```ignore
/// reg.add(schema, handler(|ctx, a| async move { Ok(json!({"ok": true})) }));
/// ```
pub fn handler<F, Fut>(f: F) -> ToolHandler
where
    F: Fn(ToolCtx, Args) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value>> + Send + 'static,
{
    Arc::new(move |ctx, args| Box::pin(f(ctx, args)))
}

/// Guard used by tools that must not be reachable when a subsystem is disabled.
pub fn disabled(subsystem: &str) -> ToolError {
    ToolError::not_supported(format!("{subsystem} is not enabled on this server"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dummy_schema(name: &'static str) -> ToolSchema {
        ToolSchema::new(name, "test tool").req_str("mount_id", "Project/volume id.")
    }

    fn registry_with(names: &[&'static str]) -> ToolRegistry {
        let mut r = ToolRegistry::new();
        for n in names {
            r.add(
                dummy_schema(n),
                handler(|_ctx, a: Args| async move {
                    Ok(json!({"echo": a.str("mount_id")?}))
                }),
            );
        }
        r
    }

    #[test]
    fn registration_order_is_preserved_in_list() {
        let r = registry_with(&["fs.read", "fs.write", "admin.list_projects"]);
        let p = r.list_payload();
        let names: Vec<&str> =
            p["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["fs.read", "fs.write", "admin.list_projects"]);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn resolve_exact_and_underscore_variant() {
        let r = registry_with(&["admin.list_projects"]);
        assert!(r.resolve("admin.list_projects").is_some());
        assert!(r.resolve("admin_list_projects").is_some());
        assert!(r.resolve("admin.nope").is_none());
    }

    #[test]
    #[should_panic(expected = "duplicate tool registration")]
    fn duplicate_registration_panics() {
        registry_with(&["fs.read", "fs.read"]);
    }

    #[test]
    fn list_entry_contains_schema() {
        let r = registry_with(&["fs.read"]);
        let p = r.list_payload();
        let t = &p["tools"][0];
        assert_eq!(t["name"], "fs.read");
        assert_eq!(t["inputSchema"]["type"], "object");
        assert_eq!(t["inputSchema"]["required"], json!(["mount_id"]));
    }
}
