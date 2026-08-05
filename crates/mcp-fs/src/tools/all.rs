//! The single registration entry point the composition root calls.
//!
//! Order matters: `tools/list` renders the registry in registration order, so the
//! `fs.*` families come first, then `admin.*`, then the git families when git is
//! enabled, and the optional web and context7 families last.

/// Which optional tool families to register.
pub struct EnabledFeatures {
    pub git: bool,
    pub web: bool,
    pub context7: bool,
}

/// Register every tool: the fs.* families, then admin.*, then the optional families.
pub fn register_all(
    reg: &mut crate::mcp::ToolRegistry,
    features: &EnabledFeatures,
    config: &crate::config::ServerConfig,
) {
    super::register_fs(reg);
    super::admin::register(reg);
    if features.git {
        super::git::register(reg);
        super::git_auth::register(reg);
    }
    if features.web {
        super::web::register(reg, &config.web);
    }
    if features.context7 {
        super::context7::register(reg, &config.context7);
    }
}

#[cfg(test)]
mod tests {
    use crate::mcp::ToolRegistry;

    use super::EnabledFeatures;

    /// With git disabled the git families must be absent, not merely unreachable:
    /// an LLM must not see a tool it cannot call.
    #[test]
    fn admin_tools_register_without_git() {
        let mut reg = ToolRegistry::new();
        super::super::admin::register(&mut reg);
        assert_eq!(reg.len(), 8);
        assert!(reg.resolve("git.init").is_none());
        assert!(reg.resolve("git.auth").is_none());
    }

    #[test]
    fn the_git_families_add_fourteen_tools() {
        let mut reg = ToolRegistry::new();
        super::super::admin::register(&mut reg);
        super::super::git::register(&mut reg);
        super::super::git_auth::register(&mut reg);
        assert_eq!(reg.len(), 8 + 11 + 3);
        assert!(reg.resolve("git.remote_clone").is_some());
        assert!(reg.resolve("git.auth_revoke").is_some());
    }

    #[test]
    fn web_and_context7_tools_register_when_enabled() {
        let mut reg = ToolRegistry::new();
        let features = EnabledFeatures { git: false, web: true, context7: true };
        let config = crate::config::ServerConfig::default();
        super::register_all(&mut reg, &features, &config);
        // 33 fs + 8 admin + 4 web + 2 context7 = 47
        assert_eq!(reg.len(), 47);
        assert!(reg.resolve("web.search").is_some());
        assert!(reg.resolve("context7.resolve_library_id").is_some());
    }

    /// Whole surface parity gate for the 22 tools of this agent: every `admin.*`,
    /// `git.*` and `git.auth*` schema and description is compared to the
    /// `tools/list` captured from the running C# server, the serialized string
    /// included, so a property key ORDER change fails too.
    ///
    /// The capture lives at the repo root, outside the crate, so the check is
    /// skipped with a message when it is absent; the per family tests still pin
    /// every schema inline.
    #[test]
    fn every_admin_and_git_schema_matches_the_captured_csharp_tools_list() {
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
        super::super::admin::register(&mut reg);
        super::super::git::register(&mut reg);
        super::super::git_auth::register(&mut reg);

        let mut compared = 0;
        for tool in live {
            let name = tool["name"].as_str().unwrap();
            if !(name.starts_with("admin.") || name.starts_with("git.")) {
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
        assert_eq!(compared, 22, "the capture must cover all 8 admin.* and 14 git.* tools");
    }

    /// Registration order is the `tools/list` order.
    #[test]
    fn admin_comes_before_git() {
        let mut reg = ToolRegistry::new();
        super::super::admin::register(&mut reg);
        super::super::git::register(&mut reg);
        let names = reg.names();
        let first_git = names.iter().position(|n| n.starts_with("git.")).unwrap();
        let last_admin = names.iter().rposition(|n| n.starts_with("admin.")).unwrap();
        assert!(last_admin < first_git);
    }
}
