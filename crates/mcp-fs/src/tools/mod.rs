//! The MCP tool layer: one module per tool family, each declaring its JSON
//! Schema and dispatching to the engine in [`crate::core::fs_ops`] (or, for the
//! document family, [`crate::docs`]).
//!
//! Nothing here reimplements filesystem behaviour. A handler does exactly three
//! things: authorize the caller on the volume, normalize the path arguments, and
//! call the engine. That keeps the MCP surface and the REST data plane on one
//! implementation, which is the whole point of the split.
//!
//! Parity notes:
//! * parameter names are snake_case so the generated schema matches the C#
//!   surface byte for byte (see [`crate::mcp::schema`]),
//! * parameter descriptions are the LLM facing docs and are copied verbatim from
//!   the live C# server (captured in `TOOL_CONTRACT.txt` / `parity-golden.json`),
//! * `authorize` runs before any storage access, and before path normalization,
//!   so a non member gets `ERR_FORBIDDEN` rather than a path error.

pub mod admin;
pub mod all;
pub mod context7;
pub mod db;
pub mod document;
pub mod edit;
pub mod git;
pub mod git_auth;
pub mod lifecycle;
pub mod listing;
pub mod metadata;
pub mod read;
pub mod search;
pub mod sqlite;
pub mod web;
pub mod write;

use crate::errors::Result;
use crate::mcp::Args;
use crate::mcp::registry::ToolCtx;
use crate::storage::VolumeClient;
use std::sync::Arc;

/// Register every fs.* tool. Families are added in a stable order so `tools/list`
/// output is deterministic.
pub fn register_fs(reg: &mut crate::mcp::ToolRegistry) {
    read::register(reg);
    write::register(reg);
    edit::register(reg);
    search::register(reg);
    listing::register(reg);
    metadata::register(reg);
    lifecycle::register(reg);
    document::register(reg);
}

/// Read `mount_id`, enforce project membership, open the volume.
///
/// Every `fs.*` handler that touches bytes starts here: the port of the C#
/// `ToolContext.ClientAsync`, which is also where the ACL check lives.
pub(crate) async fn volume(ctx: &ToolCtx, a: &Args) -> Result<(String, Arc<VolumeClient>)> {
    let mount = a.str("mount_id")?;
    ctx.state.authorize(&mount, &ctx.person).await?;
    let client = ctx.state.stores.client(&mount).await?;
    Ok((mount, client))
}

/// Membership gate for the two tools that never open a volume
/// (`fs.list_allowed_roots`, `fs.audit_log`): they still take `mount_id` and
/// still authorize, exactly like the C# `ToolContext.AuthorizeAsync`.
pub(crate) async fn authorize_only(ctx: &ToolCtx, a: &Args) -> Result<String> {
    let mount = a.str("mount_id")?;
    ctx.state.authorize(&mount, &ctx.person).await?;
    Ok(mount)
}

/// Normalize one path argument, rejecting escapes out of the volume root.
pub(crate) fn norm(ctx: &ToolCtx, a: &Args, name: &str) -> Result<String> {
    ctx.state.safety.normalize_path(&a.str(name)?)
}

/// Normalize an optional path argument that carries a default (`root`, `path`).
pub(crate) fn norm_or(ctx: &ToolCtx, a: &Args, name: &str, default: &str) -> Result<String> {
    ctx.state.safety.normalize_path(&a.str_or(name, default))
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Shared in-process harness for the family tests: a real `AppState` (SQLite
    //! metadata in a temp dir, local blobs, ACL registry in memory) plus a
    //! registry holding every fs.* tool, so tests exercise the same dispatch path
    //! the HTTP endpoint uses.

    use crate::config::ServerConfig;
    use crate::errors::Result;
    use crate::mcp::registry::ToolCtx;
    use crate::mcp::{Args, ToolRegistry};
    use crate::state::AppState;
    use crate::storage::traits::AdminBackend;
    use crate::storage::VolumeClient;
    use serde_json::Value;
    use std::sync::Arc;

    pub const PERSON: &str = "a@b.c";
    pub const MOUNT: &str = "proj";

    pub struct Harness {
        _dir: tempfile::TempDir,
        pub state: Arc<AppState>,
    }

    impl Harness {
        /// Dispatch through the registry, like `tools/call` does.
        pub async fn call(&self, name: &str, args: Value) -> Result<Value> {
            let ctx = ToolCtx { person: PERSON.to_string(), state: self.state.clone() };
            self.state
                .registry
                .call(name, ctx, Args::new(args))
                .await
                .unwrap_or_else(|| panic!("tool '{name}' is not registered"))
        }

        /// Direct volume access, to seed fixtures without going through a tool.
        pub async fn client(&self) -> Arc<VolumeClient> {
            self.state.stores.client(MOUNT).await.unwrap()
        }

        /// Seed a file and mark it read, the precondition of the edit family.
        pub async fn seed(&self, path: &str, text: &str) {
            self.client().await.write_text_atomic(path, text).await.unwrap();
            self.state.safety.record_read(PERSON, MOUNT, path);
        }
    }

    /// `PERSON` owns `MOUNT`; every fs.* tool is registered.
    pub async fn harness() -> Harness {
        harness_with(|_| {}).await
    }

    /// Same, with a hook to tweak the config (quota, read guard, hard delete).
    pub async fn harness_with(tweak: impl FnOnce(&mut ServerConfig)) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        // No public key: the harness never mints tokens, it injects the identity.
        config.auth.jwt.public_key_path = String::new();
        tweak(&mut config);
        let config = Arc::new(config);

        let admin = Arc::new(crate::storage::admin::SqliteAdminStore::in_memory().unwrap());
        admin.connect().await.unwrap();
        admin.create_project(MOUNT, PERSON).await.unwrap();

        let mut registry = ToolRegistry::new();
        super::register_fs(&mut registry);

        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(config.clone())),
            safety: Arc::new(crate::safety::SafetyManager::new(config.safety.clone())),
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            registry: Arc::new(registry),
        });
        Harness { _dir: dir, state }
    }

    /// Harness with extra registrations applied after the default `register_fs`.
    pub async fn harness_with_extra(
        extra: impl FnOnce(&mut ToolRegistry, &crate::config::ServerConfig),
    ) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.infra.meta.dir = dir.path().join("volumes").display().to_string();
        config.infra.blob.dir = dir.path().join("blobs").display().to_string();
        config.auth.jwt.public_key_path = String::new();
        let config = Arc::new(config);

        let admin = Arc::new(crate::storage::admin::SqliteAdminStore::in_memory().unwrap());
        admin.connect().await.unwrap();
        admin.create_project(MOUNT, PERSON).await.unwrap();

        let mut registry = ToolRegistry::new();
        super::register_fs(&mut registry);
        extra(&mut registry, &config);

        let state = Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(config.clone())),
            safety: Arc::new(crate::safety::SafetyManager::new(config.safety.clone())),
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            registry: Arc::new(registry),
        });
        Harness { _dir: dir, state }
    }

    /// Assert a family registers exactly the expected tool names, in order.
    pub fn assert_family(register: fn(&mut ToolRegistry), expected: &[&str]) {
        let mut reg = ToolRegistry::new();
        register(&mut reg);
        assert_eq!(reg.names(), expected);
    }

    /// Assert one tool's generated `inputSchema` equals the captured C# schema.
    pub fn assert_schema(register: fn(&mut ToolRegistry), name: &str, expected_json: &str) {
        let mut reg = ToolRegistry::new();
        register(&mut reg);
        let tool = reg.resolve(name).unwrap_or_else(|| panic!("'{name}' not registered"));
        let expected: Value = serde_json::from_str(expected_json).expect("valid expected JSON");
        assert_eq!(tool.schema.input_schema(), expected, "schema drift on {name}");
    }

    /// Assert one tool's description equals the captured C# description.
    pub fn assert_description(register: fn(&mut ToolRegistry), name: &str, expected: &str) {
        let mut reg = ToolRegistry::new();
        register(&mut reg);
        let tool = reg.resolve(name).unwrap();
        assert_eq!(tool.schema.description, expected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ToolRegistry;
    use serde_json::json;
    use testkit::{MOUNT, harness};

    /// The 33 fs.* tools of this layer, in registration order.
    const FS_TOOLS: &[&str] = &[
        "fs.read",
        "fs.read_bytes",
        "fs.read_lines",
        "fs.read_section",
        "fs.read_many",
        "fs.head",
        "fs.tail",
        "fs.count_lines",
        "fs.write",
        "fs.append",
        "fs.create_empty",
        "fs.edit",
        "fs.multi_edit",
        "fs.search_replace",
        "fs.insert_at_line",
        "fs.apply_patch",
        "fs.glob",
        "fs.grep",
        "fs.find_definition",
        "fs.find_references",
        "fs.list_dir",
        "fs.tree",
        "fs.stat",
        "fs.exists",
        "fs.hash",
        "fs.mkdir",
        "fs.delete",
        "fs.move",
        "fs.copy",
        "fs.list_allowed_roots",
        "fs.audit_log",
        "fs.extract_text",
        "fs.write_docx",
    ];

    #[test]
    fn register_fs_registers_every_family_in_order() {
        let mut reg = ToolRegistry::new();
        register_fs(&mut reg);
        assert_eq!(reg.len(), 33);
        assert_eq!(reg.names(), FS_TOOLS);
    }

    #[test]
    fn every_registered_tool_is_an_fs_tool_with_a_description() {
        let mut reg = ToolRegistry::new();
        register_fs(&mut reg);
        for name in reg.names() {
            let t = reg.resolve(name).unwrap();
            assert!(name.starts_with("fs."), "{name} is not in the fs family");
            assert!(!t.schema.description.is_empty(), "{name} has no description");
            assert_eq!(t.schema.input_schema()["type"], "object");
        }
    }

    /// `mount_id` is required on every fs.* tool, including the two that never
    /// open a volume.
    #[test]
    fn mount_id_is_required_everywhere() {
        let mut reg = ToolRegistry::new();
        register_fs(&mut reg);
        for name in reg.names() {
            let schema = reg.resolve(name).unwrap().schema.input_schema();
            let required = schema["required"].as_array().expect(name);
            assert_eq!(required[0], "mount_id", "{name} must require mount_id first");
        }
    }

    #[tokio::test]
    async fn a_non_member_is_forbidden_before_anything_else() {
        let h = harness().await;
        let ctx = crate::mcp::registry::ToolCtx {
            person: "stranger@x.y".into(),
            state: h.state.clone(),
        };
        let err = h
            .state
            .registry
            .call("fs.read", ctx, Args::new(json!({"mount_id": MOUNT, "path": "/nope"})))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.code, crate::errors::code::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_missing_mount_id_is_an_invalid_argument() {
        let h = harness().await;
        let err = h.call("fs.stat", json!({"path": "/a.txt"})).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn an_unknown_project_is_not_found() {
        let h = harness().await;
        let err = h.call("fs.exists", json!({"mount_id": "nope", "path": "/"})).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::PROJECT_NOT_FOUND);
    }

    /// Whole surface parity gate: every fs.* schema and description is compared to
    /// the `tools/list` captured from the running C# server, serialized string
    /// included, so a key ORDER change fails too.
    ///
    /// The capture lives at the repo root and is not part of the crate, so the
    /// check is skipped (with a message) when it is not there; the per family
    /// tests still pin the schemas inline.
    #[test]
    fn every_fs_schema_matches_the_captured_csharp_tools_list() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../parity-golden.json");
        let Ok(raw) = std::fs::read_to_string(path) else {
            eprintln!("skipped: {path} is absent");
            return;
        };
        let golden: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let live = golden["steps"]["tools_list"]["body"]["result"]["tools"]
            .as_array()
            .expect("the capture must contain a tools/list step");

        let mut reg = ToolRegistry::new();
        register_fs(&mut reg);
        let mut compared = 0;
        for tool in live {
            let name = tool["name"].as_str().unwrap();
            if !name.starts_with("fs.") {
                continue;
            }
            compared += 1;
            let mine = reg.resolve(name).unwrap_or_else(|| panic!("{name} is not registered"));
            assert_eq!(
                mine.schema.description,
                tool["description"].as_str().unwrap(),
                "description drift on {name}"
            );
            assert_eq!(mine.schema.input_schema(), tool["inputSchema"], "schema drift on {name}");
            assert_eq!(
                serde_json::to_string(&mine.schema.input_schema()).unwrap(),
                serde_json::to_string(&tool["inputSchema"]).unwrap(),
                "property key order drift on {name}"
            );
        }
        assert_eq!(compared, 33, "the capture must cover all 33 fs.* tools");
    }
}

pub use all::register_all;
