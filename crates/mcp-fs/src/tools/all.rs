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
    pub sqlite: bool,
    pub db: bool,
    pub doc: bool,
    pub search: bool,
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
    if features.sqlite {
        super::sqlite::register(reg, &config.sqlite);
    }
    if features.db {
        super::db::register(reg, &config.db);
    }
    if features.doc {
        super::doc::register(reg, &config.doc);
        super::editor::register(reg);
    }
    if features.search {
        super::search_semantic::register(reg, &config.search);
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
        assert_eq!(reg.len(), 10);
        assert!(reg.resolve("git.init").is_none());
        assert!(reg.resolve("git.auth").is_none());
    }

    #[test]
    fn the_git_families_add_fourteen_tools() {
        let mut reg = ToolRegistry::new();
        super::super::admin::register(&mut reg);
        super::super::git::register(&mut reg);
        super::super::git_auth::register(&mut reg);
        assert_eq!(reg.len(), 10 + 11 + 3);
        assert!(reg.resolve("git.remote_clone").is_some());
        assert!(reg.resolve("git.auth_revoke").is_some());
    }

    #[test]
    fn web_and_context7_tools_register_when_enabled() {
        let mut reg = ToolRegistry::new();
        let features = EnabledFeatures { git: false, web: true, context7: true, sqlite: false, db: false, doc: false, search: false };
        let config = crate::config::ServerConfig::default();
        super::register_all(&mut reg, &features, &config);
        // 35 fs + 10 admin + 5 web + 2 context7 = 52
        assert_eq!(reg.len(), 52);
        assert!(reg.resolve("web.search").is_some());
        assert!(reg.resolve("context7.resolve_library_id").is_some());
    }

    #[test]
    fn all_features_enabled_count() {
        let mut reg = ToolRegistry::new();
        let features = EnabledFeatures { git: true, web: true, context7: true, sqlite: true, db: true, doc: true, search: false };
        let config = crate::config::ServerConfig::default();
        super::register_all(&mut reg, &features, &config);
        // 35 fs + 10 admin + 14 git + 5 web + 2 context7 + 8 sqlite + 5 db = 79
        // + 2 doc.to_docx / doc.to_pptx if pandoc is in PATH, 0 otherwise
        // + 3 doc.open_editor / doc.close_editor / doc.list_editors always
        let doc_count = if which::which("pandoc").is_ok() { 2 } else { 0 };
        assert_eq!(reg.len(), 79 + doc_count + 3);
    }

    #[test]
    fn all_features_with_search_adds_four_tools() {
        let mut reg = ToolRegistry::new();
        let features = EnabledFeatures { git: true, web: true, context7: true, sqlite: true, db: true, doc: true, search: true };
        let config = crate::config::ServerConfig::default();
        super::register_all(&mut reg, &features, &config);
        let doc_count = if which::which("pandoc").is_ok() { 2 } else { 0 };
        // Base 79 + doc + 3 editor + 4 search.*
        assert_eq!(reg.len(), 79 + doc_count + 3 + 4);
        assert!(reg.resolve("search.index").is_some());
        assert!(reg.resolve("search.query").is_some());
        assert!(reg.resolve("search.delete").is_some());
        assert!(reg.resolve("search.status").is_some());
    }

    /// Whole surface gate for the 24 tools of this agent: every `admin.*`,
    /// `git.*` and `git.auth*` schema and description is compared to the frozen
    /// contract, the serialized string included, so a property key ORDER change
    /// fails too.
    ///
    /// The contract lives at the repo root, outside the crate, so the check is
    /// skipped with a message when it is absent; the per family tests still pin
    /// every schema inline.
    #[test]
    fn every_admin_and_git_schema_matches_the_frozen_tool_contract() {
        let mut reg = ToolRegistry::new();
        super::super::admin::register(&mut reg);
        super::super::git::register(&mut reg);
        super::super::git_auth::register(&mut reg);
        super::super::contract_golden::assert_family(
            &reg,
            |name| name.starts_with("admin.") || name.starts_with("git."),
            24,
            "admin.* and git.* tools",
        );
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
