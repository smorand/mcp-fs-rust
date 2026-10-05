//! `admin.*` tools: project lifecycle and membership.
//!
//! Port of the C# `Tools/AdminTools.cs`. Authority ladder, unchanged from the C#:
//! platform admin (`auth.admins`) > project owner > member.
//!
//! | tool                     | gate                                            |
//! |--------------------------|-------------------------------------------------|
//! | create_project           | platform admin                                  |
//! | list_all_projects        | platform admin                                  |
//! | list_users               | platform admin                                  |
//! | delete_project           | owner or platform admin                         |
//! | add_member               | owner or platform admin                         |
//! | remove_member            | owner or platform admin                         |
//! | list_members             | member, or platform admin (project must exist)  |
//! | list_projects            | none, scoped to the caller's own projects       |
//! | list_deleted_projects    | none, scoped to the caller's own projects,      |
//! |                          | platform admin sees all (SPEC-0014 US-0010)     |
//! | undelete_project         | owner or platform admin (SPEC-0014 US-0011)     |
//! | set_index_mode           | owner or platform admin                         |
//! | get_index_mode           | member, or platform admin (project must exist)  |
//! | set_purge_config         | owner or platform admin                         |
//! | set_project_quota        | platform admin only (SPEC-0010)                 |
//!
//! `create_project` provisions the volume and rolls the ACL row back if that
//! fails, so a project row never points at a volume that was never created.

use crate::errors::{Result, ToolError};
#[cfg(test)]
use crate::git::GitRepoStore;
#[cfg(test)]
use crate::storage::admin::validate_project_id;
#[cfg(test)]
use crate::storage::traits::IndexMode;
use crate::tools::registry_support::ToolCtx;
#[cfg(test)]
use crate::tools::registry_support::handler;
#[cfg(test)]
use crate::tools::registry_support::{ToolRegistry, ToolSchema};
use crate::util::normalize_identity;
use serde_json::{Value, json};
#[cfg(test)]
use std::str::FromStr;
#[cfg(test)]
use std::sync::Arc;

/// Register the thirteen `admin.*` tools.
///
/// Test-only: the live MCP surface dispatches through `mcp::server::McpServer`'s
/// `rmcp` tool router, never through this registry.
#[cfg(test)]
pub(crate) fn register(reg: &mut ToolRegistry) {
    register_with(reg, None);
}

/// Registration with an injected git store, used by tests (and by any caller that
/// already owns a store). `None` resolves the process wide store lazily, and only
/// when `git.enabled`, so a git free deployment never touches the git state dirs.
#[cfg(test)]
pub(crate) fn register_with(reg: &mut ToolRegistry, git: Option<Arc<GitRepoStore>>) {
    reg.add(
        ToolSchema::new(
            "admin.create_project",
            "Create a project for a designated owner and provision its volume (platform admin only).",
        )
        .req_str(
            "project_id",
            "New project id: 3 to 32 chars, lowercase letters, digits, hyphens, alphanumeric bounds.",
        )
        .req_str("owner", "Person id who owns the new project.")
        .read_only(false)
        .destructive(false)
        .idempotent(false)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            let owner = a.str("owner")?;
            create_project(&ctx, &project_id, &owner).await
        }),
    );

    let git_for_delete = git.clone();
    reg.add(
        ToolSchema::new(
            "admin.delete_project",
            "Delete a project and recursively tear down its volume (owner or platform admin).",
        )
        .req_str("project_id", "Id of the project to delete.")
        .read_only(false)
        .destructive(true)
        .idempotent(true)
        .open_world(false),
        handler(move |ctx: ToolCtx, a| {
            let git = git_for_delete.clone();
            async move {
                let project_id = a.str("project_id")?;
                delete_project(&ctx, &project_id, git).await
            }
        }),
    );

    reg.add(
        ToolSchema::new("admin.list_projects", "List projects the caller can access.")
            .read_only(true)
            .idempotent(true)
            .open_world(false),
        handler(|ctx: ToolCtx, _a| async move { list_projects(&ctx).await }),
    );

    reg.add(
        ToolSchema::new("admin.list_all_projects", "List every project (platform admin only).")
            .read_only(true)
            .idempotent(true)
            .open_world(false),
        handler(|ctx: ToolCtx, _a| async move { list_all_projects(&ctx).await }),
    );

    reg.add(
        ToolSchema::new(
            "admin.list_deleted_projects",
            "List soft-deleted projects visible to the caller, with a countdown to \
             permanent removal (membership-filtered, platform admin sees all).",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, _a| async move { list_deleted_projects(&ctx).await }),
    );

    reg.add(
        ToolSchema::new(
            "admin.undelete_project",
            "Clear a project's soft-delete flag, immediately restoring access (owner or platform admin).",
        )
        .req_str("project_id", "Id of the soft-deleted project to restore.")
        .read_only(false)
        .destructive(false)
        .idempotent(false)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            undelete_project(&ctx, &project_id).await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.list_users",
            "List every known person and platform admins (platform admin only).",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, _a| async move { list_users(&ctx).await }),
    );

    reg.add(
        ToolSchema::new("admin.add_member", "Add a person to a project (owner or platform admin).")
            .req_str("project_id", "Id of the project to add the member to.")
            .req_str("person", "Person id to add as a member.")
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            let person = a.str("person")?;
            add_member(&ctx, &project_id, &person).await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.remove_member",
            "Remove a person from a project (owner or platform admin).",
        )
        .req_str("project_id", "Id of the project to remove the member from.")
        .req_str("person", "Person id to remove from the project.")
        .read_only(false)
        .destructive(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            let person = a.str("person")?;
            remove_member(&ctx, &project_id, &person).await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.list_members",
            "List members of a project (member or platform admin).",
        )
        .req_str("project_id", "Id of the project whose members are listed.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            list_members(&ctx, &project_id).await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.set_index_mode",
            "Set the search index mode for a project (owner or platform admin). \
             none=no index, bm25=full-text only, rag=vector only, both=full-text and vector. \
             Switching to an active mode triggers an initial full index of existing files in \
             the background. Switching to none wipes the index immediately.",
        )
        .req_str("project_id", "Id of the project whose search index mode is set.")
        .req_str(
            "mode",
            "New index mode: none, bm25, rag, or both. rag and both need a configured \
             embedding endpoint. Any change to a different mode wipes the current index \
             before rebuilding it, so setting the mode a project already has is a no-op \
             that keeps the index intact.",
        )
        .read_only(false)
        .destructive(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            let mode = a.str("mode")?;
            set_index_mode(&ctx, &project_id, &mode).await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.get_index_mode",
            "Get the current search index mode for a project (member or platform admin).",
        )
        .req_str("project_id", "Id of the project whose search index mode is read.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            get_index_mode(&ctx, &project_id).await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.set_purge_config",
            "Configure auto-purge for a project: enable flag, internal-vs-external driver \
             choice, and the two independent retention thresholds (owner or platform admin). \
             Either retention value may be absent to disable that axis independently.",
        )
        .req_str("project_id", "Id of the project whose auto-purge configuration is set.")
        .req_bool("autopurge_enabled", "Whether auto-purge is enabled for this project.")
        .req_bool(
            "use_internal_purge",
            "Whether the internal purge driver is used, as opposed to an external one.",
        )
        .opt_nullable_uint(
            "file_retention_days",
            "Days of inactivity before a file is purged. Must be > 0 when present; absent \
             disables this axis.",
        )
        .opt_nullable_uint(
            "project_retention_days",
            "Days after soft-delete before a project is purged. Must be > 0 when present; \
             absent disables this axis.",
        )
        .read_only(false)
        .destructive(false)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            let autopurge_enabled = a.bool("autopurge_enabled")?;
            let use_internal_purge = a.bool("use_internal_purge")?;
            let file_retention_days = a.opt_i64("file_retention_days")?;
            let project_retention_days = a.opt_i64("project_retention_days")?;
            set_purge_config(
                &ctx,
                &project_id,
                autopurge_enabled,
                use_internal_purge,
                file_retention_days,
                project_retention_days,
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new(
            "admin.set_project_quota",
            "Set or clear a project's maximum storage size in megabytes (platform admin \
             only). max_mb absent or null clears the quota (unlimited); max_mb must be > 0 \
             when present. Lowering the quota below current usage is accepted and evicts \
             nothing.",
        )
        .req_str("project_id", "Id of the project whose storage quota is set.")
        .opt_nullable_uint(
            "max_mb",
            "Maximum storage size in megabytes. Absent or null clears the quota \
             (unlimited). Must be > 0 when present.",
        )
        .read_only(false)
        .destructive(false)
        .idempotent(true)
        .open_world(false),
        handler(|ctx: ToolCtx, a| async move {
            let project_id = a.str("project_id")?;
            let max_mb = a.opt_i64("max_mb")?.map(|m| m as u32);
            set_project_quota(&ctx, &project_id, max_mb).await
        }),
    );
}

// ── implementations ─────────────────────────────────────────────────────────

#[cfg(test)]
async fn create_project(ctx: &ToolCtx, project_id: &str, owner: &str) -> Result<Value> {
    ctx.state.require_admin(&ctx.person)?;
    validate_project_id(project_id)?;
    if owner.trim().is_empty() {
        return Err(ToolError::invalid_argument("owner is required"));
    }
    let project = ctx.state.admin.create_project(project_id, owner).await?;
    if let Err(e) = ctx.state.stores.provision_volume(project_id).await {
        // Roll the ACL row back: a project whose volume does not exist would fail
        // every later fs.* call with a confusing storage error.
        let _ = ctx.state.admin.delete_project(project_id).await;
        return Err(e);
    }
    Ok(json!({
        "project_id": project.id,
        "owner": project.owner,
        "created_at": project.created_at,
    }))
}

#[cfg(test)]
async fn delete_project(
    ctx: &ToolCtx,
    project_id: &str,
    git: Option<Arc<GitRepoStore>>,
) -> Result<Value> {
    ctx.state.require_owner_or_admin(project_id, &ctx.person).await?;
    ctx.state.stores.teardown_volume(project_id).await?;
    // The C# only dropped the in process git entry, so state/git/{id}.db survived
    // and a project recreated under the same id inherited stale refs. Purge it.
    if ctx.state.config.git.enabled {
        let store = git.unwrap_or_else(|| {
            GitRepoStore::shared(ctx.state.config.clone(), ctx.state.stores.relational().clone())
        });
        store.purge_repo(project_id).await?;
    }
    ctx.state.admin.delete_project(project_id).await?;
    Ok(json!({"project_id": project_id, "deleted": true}))
}

#[cfg(test)]
async fn list_projects(ctx: &ToolCtx) -> Result<Value> {
    let person = normalize_identity(&ctx.person);
    let projects = ctx.state.admin.list_projects_for(&ctx.person).await?;
    let entries: Vec<Value> = projects
        .into_iter()
        .map(|p| {
            json!({
                "project_id": p.id,
                "owner": p.owner,
                "created_at": p.created_at,
                "index_mode": p.index_mode,
                // Caseless, unlike the C# ordinal compare: the store normalizes
                // the owner, so a mixed case caller must not be told it is not one.
                "is_owner": normalize_identity(&p.owner) == person,
            })
        })
        .collect();
    Ok(json!({"projects": entries}))
}

#[cfg(test)]
async fn list_all_projects(ctx: &ToolCtx) -> Result<Value> {
    ctx.state.require_admin(&ctx.person)?;
    let projects = ctx.state.admin.list_all_projects().await?;
    let entries: Vec<Value> = projects
        .into_iter()
        .map(|p| {
            json!({
                "project_id": p.id,
                "owner": p.owner,
                "created_at": p.created_at,
                "index_mode": p.index_mode,
            })
        })
        .collect();
    Ok(json!({"projects": entries}))
}

/// Membership-filtered the same way `list_projects` filters (DEC-012), not
/// platform-admin-only: a platform admin additionally sees every soft-deleted
/// project, matching the verified `admin.list_projects` convention rather than
/// gating the whole tool behind `require_admin`.
pub(crate) async fn list_deleted_projects(ctx: &ToolCtx) -> Result<Value> {
    let person = normalize_identity(&ctx.person);
    let is_admin = ctx.state.is_admin(&ctx.person);
    let grace_days = ctx.state.config.safety.project_purge_grace_days;
    let now = crate::util::now_unix();
    let mut entries = Vec::new();
    for (project_id, deleted_at) in ctx.state.admin.list_soft_deleted_projects().await? {
        if !is_admin && !ctx.state.admin.is_member(&project_id, &person).await? {
            continue;
        }
        let owner =
            ctx.state.admin.get_project(&project_id).await?.map(|p| p.owner).unwrap_or_default();
        let deleted_unix = parse_deleted_at(&deleted_at)?;
        let days_elapsed = ((now - deleted_unix) / SECONDS_PER_DAY).floor() as i64;
        let days_until_permanent_removal = grace_days - days_elapsed;
        entries.push(json!({
            "project_id": project_id,
            "owner": owner,
            "deleted_at": deleted_at,
            "days_until_permanent_removal": days_until_permanent_removal,
        }));
    }
    Ok(json!({"deleted_projects": entries}))
}

/// SPEC-0014 US-0011: clears `deleted_at`, restoring normal access. Authorization
/// reuses `require_owner_or_admin` verbatim, which per US-0004 is exempted from
/// the `deleted_at` filter, so it still works on a soft-deleted row.
pub(crate) async fn undelete_project(ctx: &ToolCtx, project_id: &str) -> Result<Value> {
    ctx.state.require_owner_or_admin(project_id, &ctx.person).await?;
    ctx.state.admin.undelete_project(project_id).await?;
    Ok(json!({"project_id": project_id, "undeleted": true}))
}

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Parses `project.deleted_at` (RFC 3339, the format [`crate::util::now_iso`]
/// writes) into fractional Unix seconds.
fn parse_deleted_at(iso: &str) -> Result<f64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.timestamp_micros() as f64 / 1_000_000.0)
        .map_err(|e| ToolError::internal(format!("invalid deleted_at timestamp '{iso}': {e}")))
}

#[cfg(test)]
async fn list_users(ctx: &ToolCtx) -> Result<Value> {
    ctx.state.require_admin(&ctx.person)?;
    // BTreeSet gives the C# `OrderBy(p, StringComparer.Ordinal)` plus its dedup.
    let mut persons: std::collections::BTreeSet<String> =
        ctx.state.admin.list_all_persons().await?.into_iter().collect();
    for a in &ctx.state.config.auth.admins {
        // An admin who owns no project must still be listed.
        persons.insert(a.clone());
    }
    let users: Vec<Value> = persons
        .into_iter()
        .map(|p| {
            // Caseless, unlike the C# HashSet lookup: is_admin must agree with the
            // check that actually authorizes the call.
            let is_admin = ctx.state.is_admin(&p);
            json!({"person": p, "is_admin": is_admin})
        })
        .collect();
    Ok(json!({"users": users}))
}

#[cfg(test)]
async fn add_member(ctx: &ToolCtx, project_id: &str, person: &str) -> Result<Value> {
    ctx.state.require_owner_or_admin(project_id, &ctx.person).await?;
    let member = ctx.state.admin.add_member(project_id, person, &ctx.person).await?;
    Ok(json!({
        "project_id": project_id,
        "person": member.person,
        "role": member.role,
    }))
}

#[cfg(test)]
async fn remove_member(ctx: &ToolCtx, project_id: &str, person: &str) -> Result<Value> {
    ctx.state.require_owner_or_admin(project_id, &ctx.person).await?;
    ctx.state.admin.remove_member(project_id, person).await?;
    // `person` is echoed as sent, exactly like the C#, not as stored.
    Ok(json!({"project_id": project_id, "person": person, "removed": true}))
}

#[cfg(test)]
async fn list_members(ctx: &ToolCtx, project_id: &str) -> Result<Value> {
    // Not `require_owner_or_admin`: a plain member may list, an owner is not needed.
    // A platform admin skips membership but still needs the project to exist.
    if ctx.state.is_admin(&ctx.person) {
        if ctx.state.admin.get_project(project_id).await?.is_none() {
            return Err(ToolError::project_not_found(project_id));
        }
    } else {
        ctx.state.admin.require_member(project_id, &ctx.person).await?;
    }
    let members = ctx.state.admin.list_members(project_id).await?;
    let entries: Vec<Value> = members
        .into_iter()
        .map(|m| json!({"person": m.person, "role": m.role, "added_by": m.added_by}))
        .collect();
    Ok(json!({"project_id": project_id, "members": entries}))
}

/// Change a project's index mode, wiping and rebuilding the index to match.
///
/// The wipe finishes before the tool answers; the rebuild does not, so
/// `reindex_started: true` means "a background pass is running", not "the volume
/// is searchable". Poll `search.status` to watch it fill.
#[cfg(test)]
async fn set_index_mode(ctx: &ToolCtx, project_id: &str, mode: &str) -> Result<Value> {
    ctx.state.require_owner_or_admin(project_id, &ctx.person).await?;
    let new_mode = IndexMode::from_str(mode)?;

    let backend = ctx.state.search.as_ref().ok_or_else(|| {
        ToolError::not_supported("search is not enabled; set search.enabled: true in config")
    })?;
    // A vector mode without an embedding endpoint would index nothing and report
    // success, so it is refused at the gate rather than discovered in the logs.
    if new_mode.needs_embedding() && ctx.state.config.search.embedding.endpoint.is_empty() {
        return Err(ToolError::invalid_argument(format!(
            "index mode '{new_mode}' requires search.embedding.endpoint to be configured"
        )));
    }

    let old_mode = ctx.state.admin.get_index_mode(project_id).await?;
    // Stored before the wipe: a crash between the two leaves the mode recorded
    // and the index stale, which one more set_index_mode call repairs. The
    // reverse would leave a wiped index that nothing ever rebuilds.
    ctx.state.admin.set_index_mode(project_id, new_mode).await?;

    let client = ctx.state.stores.client(project_id).await?;
    let reindex_started = crate::search::indexer::ProjectIndexer::new(backend)
        .on_mode_change(project_id, old_mode, new_mode, client)
        .await?;

    Ok(json!({
        "project_id": project_id,
        "index_mode": new_mode,
        "previous_mode": old_mode,
        "reindex_started": reindex_started,
    }))
}

#[cfg(test)]
async fn get_index_mode(ctx: &ToolCtx, project_id: &str) -> Result<Value> {
    // Same gate as list_members: a plain member may read, and a platform admin
    // skips membership but still needs the project to exist.
    if ctx.state.is_admin(&ctx.person) {
        if ctx.state.admin.get_project(project_id).await?.is_none() {
            return Err(ToolError::project_not_found(project_id));
        }
    } else {
        ctx.state.admin.require_member(project_id, &ctx.person).await?;
    }
    let mode = ctx.state.admin.get_index_mode(project_id).await?;
    Ok(json!({"project_id": project_id, "index_mode": mode}))
}

#[cfg(test)]
async fn set_purge_config(
    ctx: &ToolCtx,
    project_id: &str,
    autopurge_enabled: bool,
    use_internal_purge: bool,
    file_retention_days: Option<i64>,
    project_retention_days: Option<i64>,
) -> Result<Value> {
    ctx.state.require_owner_or_admin(project_id, &ctx.person).await?;
    if file_retention_days == Some(0) || file_retention_days.is_some_and(|d| d < 0) {
        return Err(ToolError::invalid_argument("file_retention_days must be > 0"));
    }
    if project_retention_days == Some(0) || project_retention_days.is_some_and(|d| d < 0) {
        return Err(ToolError::invalid_argument("project_retention_days must be > 0"));
    }
    let config = crate::storage::traits::PurgeConfig {
        autopurge_enabled,
        use_internal_purge,
        file_retention_days,
        project_retention_days,
    };
    ctx.state.admin.set_purge_config(project_id, config).await?;
    Ok(json!({
        "project_id": project_id,
        "autopurge_enabled": autopurge_enabled,
        "use_internal_purge": use_internal_purge,
        "file_retention_days": file_retention_days,
        "project_retention_days": project_retention_days,
    }))
}

/// SPEC-0010 FR-NEW-002/003/004/013/014: platform-admin only, checked before
/// project existence (FR-NEW-014).
#[cfg(test)]
async fn set_project_quota(ctx: &ToolCtx, project_id: &str, max_mb: Option<u32>) -> Result<Value> {
    ctx.state.require_admin(&ctx.person)?;
    if max_mb == Some(0) {
        return Err(ToolError::invalid_argument("max_mb must be > 0"));
    }
    let quota_bytes = max_mb.map(|m| i64::from(m) * 1_048_576);
    ctx.state.admin.set_quota(project_id, quota_bytes).await?;
    Ok(json!({
        "project_id": project_id,
        "max_mb": max_mb,
    }))
}

// ── shared test fixtures ────────────────────────────────────────────────────

/// Fixtures shared by the `admin.*`, `git.*` and `git.auth*` test modules: a real
/// `AppState` over temp dirs plus an in memory ACL store.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::config::ServerConfig;
    use crate::errors::Result;
    use crate::identity::IdentityResolver;
    use crate::safety::SafetyManager;
    use crate::state::AppState;
    use crate::storage::StoreManager;
    use crate::storage::admin::RelationalAdminStore;
    use crate::storage::traits::AdminBackend;
    use crate::tools::registry_support::ToolCtx;
    use crate::tools::registry_support::{Args, ToolRegistry};
    use serde_json::Value;
    use std::sync::Arc;

    pub(crate) const ADMIN: &str = "admin@example.com";

    pub(crate) struct Fixture {
        /// Kept alive: dropping it deletes the state dirs.
        pub dir: tempfile::TempDir,
        pub state: Arc<AppState>,
        /// Concrete handle to the same store `state.admin` wraps, kept around so
        /// tests can reach backdoor seeding methods (`seed_deleted_at_for_test`)
        /// that are not part of the `AdminBackend` trait object.
        pub admin_store: Arc<RelationalAdminStore>,
    }

    impl Fixture {
        pub async fn new() -> Self {
            Self::with_config(|_| {}).await
        }

        pub async fn with_config(tweak: impl FnOnce(&mut ServerConfig)) -> Self {
            Self::with_config_and_search(tweak, None).await
        }

        /// Same, with a search backend injected, for the index mode tools.
        pub async fn with_config_and_search(
            tweak: impl FnOnce(&mut ServerConfig),
            search: Option<Arc<dyn crate::search::SearchBackend>>,
        ) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut config = ServerConfig::default();
            config.infra.meta.dir = dir.path().join("state/volumes").display().to_string();
            config.infra.blob.dir = dir.path().join("state/blobs").display().to_string();
            config.infra.admin.path = dir.path().join("state/admin.db").display().to_string();
            config.auth.admins = vec![ADMIN.to_string()];
            tweak(&mut config);
            let config = Arc::new(config);

            let admin_store = Arc::new(RelationalAdminStore::in_memory().await.unwrap());
            admin_store.connect().await.unwrap();

            let state = Arc::new(AppState {
                config: config.clone(),
                admin: admin_store.clone(),
                stores: Arc::new(StoreManager::new(
                    config.clone(),
                    crate::storage::test_registry(),
                )),
                safety: Arc::new(SafetyManager::new(
                    config.safety.clone(),
                    crate::storage::meta::max_path_len(&config.infra.meta.backend),
                )),
                identity: Arc::new(IdentityResolver::new(&config.auth)),
                editors: Arc::new(crate::tools::editor::EditorRegistry::new()),
                doc_service: crate::docs::service::from_config(&config.doc_service).unwrap(),
                search,
            });
            Self { dir, state, admin_store }
        }

        pub fn ctx(&self, person: &str) -> ToolCtx {
            ToolCtx { person: person.to_string(), state: self.state.clone() }
        }

        /// Create a project directly through the store, bypassing the admin gate.
        pub async fn seed_project(&self, project_id: &str, owner: &str) {
            self.state.admin.create_project(project_id, owner).await.unwrap();
            self.state.stores.provision_volume(project_id).await.unwrap();
        }

        /// Creates `project_id`, soft-deletes it, and backdates `deleted_at` to
        /// `now - age_seconds` so a countdown test can pin the exact age without
        /// racing the wall clock (same approach `purge.rs`'s `seed_soft_deleted` uses).
        pub async fn seed_soft_deleted(&self, project_id: &str, owner: &str, age_seconds: f64) {
            self.seed_project(project_id, owner).await;
            let now = crate::util::now_unix();
            let deleted_at = chrono::DateTime::from_timestamp(
                (now - age_seconds) as i64,
                (((now - age_seconds).fract()) * 1_000_000_000.0) as u32,
            )
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
            self.admin_store.seed_deleted_at_for_test(project_id, &deleted_at).await.unwrap();
        }

        pub async fn call(
            &self,
            reg: &ToolRegistry,
            person: &str,
            name: &str,
            args: Value,
        ) -> Result<Value> {
            reg.call(name, self.ctx(person), Args::new(args))
                .await
                .unwrap_or_else(|| panic!("tool '{name}' is not registered"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{ADMIN, Fixture};
    use super::*;
    use crate::errors::code;

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r);
        r
    }

    const ALL_ADMIN_TOOLS: [&str; 14] = [
        "admin.create_project",
        "admin.delete_project",
        "admin.list_projects",
        "admin.list_all_projects",
        "admin.list_deleted_projects",
        "admin.undelete_project",
        "admin.list_users",
        "admin.add_member",
        "admin.remove_member",
        "admin.list_members",
        "admin.set_index_mode",
        "admin.get_index_mode",
        "admin.set_purge_config",
        "admin.set_project_quota",
    ];

    #[test]
    fn every_admin_tool_is_registered() {
        let r = registry();
        assert_eq!(r.len(), 14);
        for name in ALL_ADMIN_TOOLS {
            assert!(r.resolve(name).is_some(), "{name} is missing");
        }
    }

    /// Schemas captured from the live C# server (TOOL_CONTRACT.txt).
    #[test]
    fn create_project_schema_matches_the_contract() {
        let r = registry();
        let s = &r.resolve("admin.create_project").unwrap().schema;
        assert_eq!(
            s.description,
            "Create a project for a designated owner and provision its volume (platform admin only)."
        );
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "project_id":{"description":"New project id: 3 to 32 chars, lowercase letters, digits, hyphens, alphanumeric bounds.","type":"string"},
                 "owner":{"description":"Person id who owns the new project.","type":"string"}},
               "required":["project_id","owner"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    #[test]
    fn add_member_schema_matches_the_contract() {
        let r = registry();
        let s = &r.resolve("admin.add_member").unwrap().schema;
        assert_eq!(s.description, "Add a person to a project (owner or platform admin).");
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "project_id":{"description":"Id of the project to add the member to.","type":"string"},
                 "person":{"description":"Person id to add as a member.","type":"string"}},
               "required":["project_id","person"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    #[test]
    fn list_projects_schema_has_no_parameters() {
        let r = registry();
        let s = &r.resolve("admin.list_projects").unwrap().schema;
        assert_eq!(s.description, "List projects the caller can access.");
        // The C# generator omits `required` entirely when there is nothing required.
        assert_eq!(
            s.input_schema(),
            serde_json::from_str::<Value>(r#"{"type":"object","properties":{}}"#).unwrap()
        );
    }

    #[test]
    fn list_users_and_list_all_projects_schemas_match_the_contract() {
        let r = registry();
        let empty: Value = serde_json::from_str(r#"{"type":"object","properties":{}}"#).unwrap();
        let u = &r.resolve("admin.list_users").unwrap().schema;
        assert_eq!(
            u.description,
            "List every known person and platform admins (platform admin only)."
        );
        assert_eq!(u.input_schema(), empty);
        let p = &r.resolve("admin.list_all_projects").unwrap().schema;
        assert_eq!(p.description, "List every project (platform admin only).");
        assert_eq!(p.input_schema(), empty);
    }

    #[test]
    fn delete_and_list_members_schemas_match_the_contract() {
        let r = registry();
        let d = &r.resolve("admin.delete_project").unwrap().schema;
        assert_eq!(
            d.input_schema()["properties"]["project_id"]["description"],
            "Id of the project to delete."
        );
        assert_eq!(d.input_schema()["required"], json!(["project_id"]));
        let m = &r.resolve("admin.list_members").unwrap().schema;
        assert_eq!(m.description, "List members of a project (member or platform admin).");
        assert_eq!(
            m.input_schema()["properties"]["project_id"]["description"],
            "Id of the project whose members are listed."
        );
    }

    #[test]
    fn remove_member_schema_matches_the_contract() {
        let r = registry();
        let s = &r.resolve("admin.remove_member").unwrap().schema;
        assert_eq!(s.description, "Remove a person from a project (owner or platform admin).");
        assert_eq!(
            s.input_schema()["properties"]["person"]["description"],
            "Person id to remove from the project."
        );
        assert_eq!(s.input_schema()["required"], json!(["project_id", "person"]));
    }

    // ── authorization ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_non_admin_cannot_create_a_project() {
        let f = Fixture::new().await;
        let r = registry();
        let e = f
            .call(
                &r,
                "nobody@test.com",
                "admin.create_project",
                json!({"project_id":"proj","owner":"nobody@test.com"}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
        assert!(e.message.contains("is not a platform admin"));
        // and nothing was created
        assert!(f.state.admin.get_project("proj").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_non_admin_cannot_list_all_projects_or_users() {
        let f = Fixture::new().await;
        let r = registry();
        for name in ["admin.list_all_projects", "admin.list_users"] {
            let e = f.call(&r, "nobody@test.com", name, json!({})).await.unwrap_err();
            assert_eq!(e.code, code::FORBIDDEN, "{name} must be admin only");
        }
    }

    #[tokio::test]
    async fn a_non_member_cannot_list_members() {
        let f = Fixture::new().await;
        f.seed_project("proj", "owner@test.com").await;
        let r = registry();

        let e = f
            .call(&r, "stranger@test.com", "admin.list_members", json!({"project_id":"proj"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
        assert!(e.message.contains("is not a member of 'proj'"));

        // a plain member may list, no ownership required
        f.state.admin.add_member("proj", "member@test.com", "owner@test.com").await.unwrap();
        let out = f
            .call(&r, "member@test.com", "admin.list_members", json!({"project_id":"proj"}))
            .await
            .unwrap();
        assert_eq!(out["members"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_platform_admin_lists_members_without_being_one() {
        let f = Fixture::new().await;
        f.seed_project("proj", "owner@test.com").await;
        let r = registry();
        let out =
            f.call(&r, ADMIN, "admin.list_members", json!({"project_id":"proj"})).await.unwrap();
        assert_eq!(out["project_id"], "proj");
        assert_eq!(out["members"][0]["person"], "owner@test.com");
        assert_eq!(out["members"][0]["role"], "owner");
        assert_eq!(out["members"][0]["added_by"], "owner@test.com");
    }

    #[tokio::test]
    async fn list_members_on_a_missing_project_is_project_not_found_for_everyone() {
        let f = Fixture::new().await;
        let r = registry();
        for person in [ADMIN, "nobody@test.com"] {
            let e = f
                .call(&r, person, "admin.list_members", json!({"project_id":"ghost"}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::PROJECT_NOT_FOUND, "for {person}");
        }
    }

    #[tokio::test]
    async fn a_member_who_is_not_the_owner_cannot_manage_membership() {
        let f = Fixture::new().await;
        f.seed_project("proj", "owner@test.com").await;
        f.state.admin.add_member("proj", "member@test.com", "owner@test.com").await.unwrap();
        let r = registry();

        for name in ["admin.add_member", "admin.remove_member"] {
            let e = f
                .call(
                    &r,
                    "member@test.com",
                    name,
                    json!({"project_id":"proj","person":"x@test.com"}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::FORBIDDEN, "{name} needs owner or admin");
        }
        let e = f
            .call(&r, "member@test.com", "admin.delete_project", json!({"project_id":"proj"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_owner_manages_membership_and_so_does_a_platform_admin() {
        let f = Fixture::new().await;
        f.seed_project("proj", "owner@test.com").await;
        let r = registry();

        let out = f
            .call(
                &r,
                "owner@test.com",
                "admin.add_member",
                json!({"project_id":"proj","person":"Bob@Test.COM"}),
            )
            .await
            .unwrap();
        assert_eq!(out["project_id"], "proj");
        assert_eq!(out["person"], "bob@test.com", "identities are normalized");
        assert_eq!(out["role"], "member");

        let out = f
            .call(
                &r,
                ADMIN,
                "admin.add_member",
                json!({"project_id":"proj","person":"carol@test.com"}),
            )
            .await
            .unwrap();
        assert_eq!(out["person"], "carol@test.com");

        let out = f
            .call(
                &r,
                ADMIN,
                "admin.remove_member",
                json!({"project_id":"proj","person":"carol@test.com"}),
            )
            .await
            .unwrap();
        assert_eq!(out, json!({"project_id":"proj","person":"carol@test.com","removed":true}));
        assert!(!f.state.admin.is_member("proj", "carol@test.com").await.unwrap());
    }

    #[tokio::test]
    async fn a_platform_admin_still_gets_project_not_found_for_a_ghost_project() {
        let f = Fixture::new().await;
        let r = registry();
        for name in ["admin.add_member", "admin.remove_member"] {
            let e = f
                .call(&r, ADMIN, name, json!({"project_id":"ghost","person":"x@test.com"}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::PROJECT_NOT_FOUND);
        }
    }

    // ── project id validation ───────────────────────────────────────────────

    #[tokio::test]
    async fn project_id_boundaries_are_enforced_before_anything_is_created() {
        let f = Fixture::new().await;
        let r = registry();

        // 3 chars ok, 32 chars ok
        for id in ["abc".to_string(), "a".repeat(32)] {
            let out = f
                .call(
                    &r,
                    ADMIN,
                    "admin.create_project",
                    json!({"project_id":id,"owner":"o@test.com"}),
                )
                .await
                .unwrap();
            assert_eq!(out["project_id"], id);
        }

        // 2 chars, 33 chars, leading hyphen, trailing hyphen, uppercase, underscore
        for bad in [
            "ab".to_string(),
            "a".repeat(33),
            "-abc".into(),
            "abc-".into(),
            "Abc".into(),
            "a_c".into(),
        ] {
            let e = f
                .call(
                    &r,
                    ADMIN,
                    "admin.create_project",
                    json!({"project_id":bad,"owner":"o@test.com"}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT, "'{bad}' must be rejected");
            assert!(e.message.contains("project_id must be 3-32 chars"));
            assert!(f.state.admin.get_project(&bad).await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn an_empty_owner_is_rejected() {
        let f = Fixture::new().await;
        let r = registry();
        let e = f
            .call(&r, ADMIN, "admin.create_project", json!({"project_id":"proj","owner":"   "}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(e.message, "owner is required");
    }

    #[tokio::test]
    async fn a_missing_argument_is_an_invalid_argument() {
        let f = Fixture::new().await;
        let r = registry();
        let e =
            f.call(&r, ADMIN, "admin.create_project", json!({"owner":"o@t.c"})).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("missing required argument 'project_id'"));
    }

    #[tokio::test]
    async fn creating_the_same_project_twice_is_project_exists() {
        let f = Fixture::new().await;
        let r = registry();
        let args = json!({"project_id":"proj","owner":"o@test.com"});
        f.call(&r, ADMIN, "admin.create_project", args.clone()).await.unwrap();
        let e = f.call(&r, ADMIN, "admin.create_project", args).await.unwrap_err();
        assert_eq!(e.code, code::PROJECT_EXISTS);
    }

    // ── round trip ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_then_list_then_delete_round_trip() {
        let f = Fixture::new().await;
        let r = registry();

        let created = f
            .call(
                &r,
                ADMIN,
                "admin.create_project",
                json!({"project_id":"round-trip","owner":"Owner@Test.COM"}),
            )
            .await
            .unwrap();
        assert_eq!(created["project_id"], "round-trip");
        assert_eq!(created["owner"], "owner@test.com");
        assert!(created["created_at"].as_str().unwrap().len() > 4);
        // the volume really exists
        assert!(f.state.config.volume_meta_path("round-trip").exists());

        // the owner sees it as theirs
        let mine = f.call(&r, "owner@test.com", "admin.list_projects", json!({})).await.unwrap();
        assert_eq!(mine["projects"][0]["project_id"], "round-trip");
        assert_eq!(mine["projects"][0]["is_owner"], true);

        // a stranger sees nothing
        let theirs =
            f.call(&r, "stranger@test.com", "admin.list_projects", json!({})).await.unwrap();
        assert_eq!(theirs["projects"].as_array().unwrap().len(), 0);

        // the platform admin sees every project, without is_owner
        let all = f.call(&r, ADMIN, "admin.list_all_projects", json!({})).await.unwrap();
        assert_eq!(all["projects"][0]["project_id"], "round-trip");
        assert!(all["projects"][0].get("is_owner").is_none());

        let deleted = f
            .call(&r, ADMIN, "admin.delete_project", json!({"project_id":"round-trip"}))
            .await
            .unwrap();
        assert_eq!(deleted, json!({"project_id":"round-trip","deleted":true}));
        assert!(f.state.admin.get_project("round-trip").await.unwrap().is_none());
        assert!(
            !f.state.config.volume_meta_path("round-trip").exists(),
            "the volume must be torn down"
        );
    }

    #[tokio::test]
    async fn deleting_a_project_purges_its_git_state_when_git_is_enabled() {
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        f.seed_project("gitproj", "owner@test.com").await;

        let git =
            Arc::new(GitRepoStore::new(f.state.config.clone(), crate::storage::test_registry()));
        git.init_repo("gitproj").await.unwrap();
        assert!(f.state.config.git_db_path("gitproj").exists());

        let mut r = ToolRegistry::new();
        register_with(&mut r, Some(git.clone()));
        f.call(&r, ADMIN, "admin.delete_project", json!({"project_id":"gitproj"})).await.unwrap();

        assert!(!f.state.config.git_db_path("gitproj").exists(), "index db purged");
        assert!(!f.state.config.git_repo_dir("gitproj").exists(), "bare repo purged");
        assert!(!git.is_initialized("gitproj").await);
    }

    #[tokio::test]
    async fn an_owner_can_delete_their_own_project() {
        let f = Fixture::new().await;
        f.seed_project("mine", "owner@test.com").await;
        let r = registry();
        f.call(&r, "owner@test.com", "admin.delete_project", json!({"project_id":"mine"}))
            .await
            .unwrap();
        assert!(f.state.admin.get_project("mine").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_users_merges_the_store_and_the_configured_admins() {
        let f = Fixture::new().await;
        f.seed_project("p1", "zoe@test.com").await;
        f.state.admin.add_member("p1", "alice@test.com", "zoe@test.com").await.unwrap();
        let r = registry();

        let out = f.call(&r, ADMIN, "admin.list_users", json!({})).await.unwrap();
        let users = out["users"].as_array().unwrap();
        let names: Vec<&str> = users.iter().map(|u| u["person"].as_str().unwrap()).collect();
        // ordinal order, and the admin appears even though it owns nothing
        assert_eq!(names, vec!["admin@example.com", "alice@test.com", "zoe@test.com"]);
        assert_eq!(users[0]["is_admin"], true);
        assert_eq!(users[1]["is_admin"], false);
    }

    // ── index mode ─────────────────────────────────────────────────────────

    /// A fixture whose search backend is a real Tantivy index over a temp dir, so
    /// the mode tools exercise a genuine wipe rather than a stub.
    async fn index_mode_fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tantivy temp dir");
        let backend: Arc<dyn crate::search::SearchBackend> =
            Arc::new(crate::search::bm25_sqlite::TantivyBm25Backend::new(
                dir.path().to_str().expect("temp dir has a utf-8 path"),
            ));
        // The backend keeps the directory alive through its own handle; leaking
        // the guard keeps it on disk for the length of the test.
        std::mem::forget(dir);
        let f = Fixture::with_config_and_search(
            |c| {
                c.search.enabled = true;
                c.search.mode = "bm25".into();
            },
            Some(backend),
        )
        .await;
        f.seed_project("proj", "owner@test.com").await;
        f
    }

    #[test]
    fn index_mode_schemas_match_the_contract() {
        let r = registry();
        let s = &r.resolve("admin.set_index_mode").unwrap().schema;
        assert_eq!(s.input_schema()["required"], json!(["project_id", "mode"]));
        assert_eq!(
            s.input_schema()["properties"]["project_id"]["description"],
            "Id of the project whose search index mode is set."
        );
        assert!(s.description.starts_with("Set the search index mode for a project"));

        let g = &r.resolve("admin.get_index_mode").unwrap().schema;
        assert_eq!(
            g.description,
            "Get the current search index mode for a project (member or platform admin)."
        );
        assert_eq!(g.input_schema()["required"], json!(["project_id"]));
    }

    #[tokio::test]
    async fn set_index_mode_owner_can_set_and_a_member_can_read() {
        let f = index_mode_fixture().await;
        f.state.admin.add_member("proj", "member@test.com", "owner@test.com").await.unwrap();
        let r = registry();

        let out = f
            .call(
                &r,
                "owner@test.com",
                "admin.set_index_mode",
                json!({"project_id":"proj","mode":"bm25"}),
            )
            .await
            .unwrap();
        assert_eq!(out["project_id"], "proj");
        assert_eq!(out["index_mode"], "bm25");
        assert_eq!(out["previous_mode"], "none");
        assert_eq!(out["reindex_started"], true);

        let read = f
            .call(&r, "member@test.com", "admin.get_index_mode", json!({"project_id":"proj"}))
            .await
            .unwrap();
        assert_eq!(read, json!({"project_id":"proj","index_mode":"bm25"}));
    }

    /// Setting the mode a project already has must not throw its index away.
    #[tokio::test]
    async fn setting_the_same_mode_twice_starts_no_reindex() {
        let f = index_mode_fixture().await;
        let r = registry();
        let args = json!({"project_id":"proj","mode":"bm25"});
        f.call(&r, "owner@test.com", "admin.set_index_mode", args.clone()).await.unwrap();
        let again = f.call(&r, "owner@test.com", "admin.set_index_mode", args).await.unwrap();
        assert_eq!(again["previous_mode"], "bm25");
        assert_eq!(again["reindex_started"], false);
    }

    #[tokio::test]
    async fn a_member_who_is_not_the_owner_cannot_set_the_index_mode() {
        let f = index_mode_fixture().await;
        f.state.admin.add_member("proj", "member@test.com", "owner@test.com").await.unwrap();
        let r = registry();
        let e = f
            .call(
                &r,
                "member@test.com",
                "admin.set_index_mode",
                json!({"project_id":"proj","mode":"bm25"}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
        // And nothing was changed.
        assert_eq!(f.state.admin.get_index_mode("proj").await.unwrap(), IndexMode::None);
    }

    #[tokio::test]
    async fn a_platform_admin_can_set_and_read_the_index_mode() {
        let f = index_mode_fixture().await;
        let r = registry();
        f.call(&r, ADMIN, "admin.set_index_mode", json!({"project_id":"proj","mode":"bm25"}))
            .await
            .unwrap();
        let out =
            f.call(&r, ADMIN, "admin.get_index_mode", json!({"project_id":"proj"})).await.unwrap();
        assert_eq!(out["index_mode"], "bm25");
    }

    #[tokio::test]
    async fn a_non_member_cannot_read_the_index_mode() {
        let f = index_mode_fixture().await;
        let r = registry();
        let e = f
            .call(&r, "stranger@test.com", "admin.get_index_mode", json!({"project_id":"proj"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_unknown_index_mode_is_an_invalid_argument() {
        let f = index_mode_fixture().await;
        let r = registry();
        let e = f
            .call(
                &r,
                "owner@test.com",
                "admin.set_index_mode",
                json!({"project_id":"proj","mode":"fancy"}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("unknown index mode 'fancy'"), "{}", e.message);
    }

    #[tokio::test]
    async fn rag_without_an_embedding_endpoint_is_an_invalid_argument() {
        let f = index_mode_fixture().await;
        let r = registry();
        for mode in ["rag", "both"] {
            let e = f
                .call(
                    &r,
                    "owner@test.com",
                    "admin.set_index_mode",
                    json!({"project_id":"proj","mode":mode}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT, "{mode} needs an endpoint");
            assert!(e.message.contains("search.embedding.endpoint"), "{}", e.message);
        }
        assert_eq!(f.state.admin.get_index_mode("proj").await.unwrap(), IndexMode::None);
    }

    #[tokio::test]
    async fn setting_a_mode_with_search_disabled_is_not_supported() {
        let f = Fixture::new().await;
        f.seed_project("proj", "owner@test.com").await;
        let r = registry();
        let e = f
            .call(
                &r,
                "owner@test.com",
                "admin.set_index_mode",
                json!({"project_id":"proj","mode":"bm25"}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code::NOT_SUPPORTED);
    }

    #[tokio::test]
    async fn index_mode_on_a_ghost_project_is_not_found() {
        let f = index_mode_fixture().await;
        let r = registry();
        for (person, name, args) in [
            (ADMIN, "admin.set_index_mode", json!({"project_id":"ghost","mode":"bm25"})),
            (ADMIN, "admin.get_index_mode", json!({"project_id":"ghost"})),
        ] {
            let e = f.call(&r, person, name, args).await.unwrap_err();
            assert_eq!(e.code, code::PROJECT_NOT_FOUND, "{name}");
        }
    }

    #[tokio::test]
    async fn the_project_listings_carry_the_index_mode() {
        let f = index_mode_fixture().await;
        let r = registry();
        f.call(
            &r,
            "owner@test.com",
            "admin.set_index_mode",
            json!({"project_id":"proj","mode":"bm25"}),
        )
        .await
        .unwrap();

        let mine = f.call(&r, "owner@test.com", "admin.list_projects", json!({})).await.unwrap();
        assert_eq!(mine["projects"][0]["index_mode"], "bm25");

        let all = f.call(&r, ADMIN, "admin.list_all_projects", json!({})).await.unwrap();
        assert_eq!(all["projects"][0]["index_mode"], "bm25");
    }

    #[tokio::test]
    async fn list_projects_needs_no_gate_and_is_caseless() {
        let f = Fixture::new().await;
        f.seed_project("mine", "owner@test.com").await;
        let r = registry();
        // mixed case caller, same person
        let out = f.call(&r, "Owner@Test.COM", "admin.list_projects", json!({})).await.unwrap();
        assert_eq!(out["projects"][0]["is_owner"], true);
    }

    // ── set_purge_config (SPEC-0014 US-0009) ────────────────────────────────────────

    #[test]
    fn set_purge_config_schema_matches_the_contract() {
        let r = registry();
        let s = &r.resolve("admin.set_purge_config").unwrap().schema;
        assert_eq!(
            s.input_schema()["required"],
            json!("project_id autopurge_enabled use_internal_purge".split(' ').collect::<Vec<_>>())
        );
    }

    /// E2E-NEW-015: owner sets config.
    #[tokio::test]
    async fn owner_sets_purge_config() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        let out = f
            .call(
                &r,
                "alice@test.com",
                "admin.set_purge_config",
                json!({
                    "project_id":"proj1",
                    "autopurge_enabled":true,
                    "use_internal_purge":true,
                    "file_retention_days":7,
                    "project_retention_days":30,
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            out,
            json!({
                "project_id":"proj1",
                "autopurge_enabled":true,
                "use_internal_purge":true,
                "file_retention_days":7,
                "project_retention_days":30,
            })
        );
        let persisted = f.state.admin.get_purge_config("proj1").await.unwrap();
        assert!(persisted.autopurge_enabled);
        assert!(persisted.use_internal_purge);
        assert_eq!(persisted.file_retention_days, Some(7));
        assert_eq!(persisted.project_retention_days, Some(30));
    }

    /// E2E-NEW-016: platform admin, not the owner, sets config.
    #[tokio::test]
    async fn platform_admin_non_owner_sets_purge_config() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        f.call(
            &r,
            ADMIN,
            "admin.set_purge_config",
            json!({
                "project_id":"proj1",
                "autopurge_enabled":true,
                "use_internal_purge":true,
                "file_retention_days":7,
                "project_retention_days":30,
            }),
        )
        .await
        .unwrap();
        let persisted = f.state.admin.get_purge_config("proj1").await.unwrap();
        assert!(persisted.autopurge_enabled);
    }

    /// E2E-NEW-017: a second happy-path variant, exercised a second time via MCP
    /// since no REST route exists for this tool.
    #[tokio::test]
    async fn owner_sets_purge_config_with_internal_purge_disabled() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        let out = f
            .call(
                &r,
                "alice@test.com",
                "admin.set_purge_config",
                json!({
                    "project_id":"proj1",
                    "autopurge_enabled":true,
                    "use_internal_purge":false,
                    "file_retention_days":14,
                    "project_retention_days":60,
                }),
            )
            .await
            .unwrap();
        assert_eq!(out["use_internal_purge"], false);
        let persisted = f.state.admin.get_purge_config("proj1").await.unwrap();
        assert!(!persisted.use_internal_purge);
        assert_eq!(persisted.file_retention_days, Some(14));
    }

    /// E2E-NEW-018: a member who is neither owner nor admin is forbidden, and
    /// the config is left unchanged.
    #[tokio::test]
    async fn non_owner_non_admin_cannot_set_purge_config() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        f.state.admin.add_member("proj1", "mallory@test.com", "alice@test.com").await.unwrap();
        let r = registry();
        let e = f
            .call(
                &r,
                "mallory@test.com",
                "admin.set_purge_config",
                json!({
                    "project_id":"proj1",
                    "autopurge_enabled":true,
                    "use_internal_purge":true,
                    "file_retention_days":7,
                    "project_retention_days":30,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
        assert!(!f.state.admin.get_purge_config("proj1").await.unwrap().autopurge_enabled);
    }

    /// E2E-NEW-019: unknown project.
    #[tokio::test]
    async fn set_purge_config_on_an_unknown_project_is_not_found() {
        let f = Fixture::new().await;
        let r = registry();
        let e = f
            .call(
                &r,
                "alice@test.com",
                "admin.set_purge_config",
                json!({
                    "project_id":"ghost",
                    "autopurge_enabled":true,
                    "use_internal_purge":true,
                    "file_retention_days":7,
                    "project_retention_days":30,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code::PROJECT_NOT_FOUND);
    }

    /// E2E-NEW-020: zero retention values are rejected, table-driven.
    #[tokio::test]
    async fn zero_retention_values_are_rejected() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        for args in [
            json!({
                "project_id":"proj1",
                "autopurge_enabled":true,
                "use_internal_purge":true,
                "file_retention_days":0,
                "project_retention_days":30,
            }),
            json!({
                "project_id":"proj1",
                "autopurge_enabled":true,
                "use_internal_purge":true,
                "file_retention_days":7,
                "project_retention_days":0,
            }),
        ] {
            let e = f.call(&r, "alice@test.com", "admin.set_purge_config", args).await.unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
        }
        // nothing was persisted
        let persisted = f.state.admin.get_purge_config("proj1").await.unwrap();
        assert!(!persisted.autopurge_enabled);
    }

    /// E2E-NEW-021: both retentions `None` is a valid, unconditioned config.
    #[tokio::test]
    async fn both_retentions_none_is_valid() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        let out = f
            .call(
                &r,
                "alice@test.com",
                "admin.set_purge_config",
                json!({
                    "project_id":"proj1",
                    "autopurge_enabled":true,
                    "use_internal_purge":false,
                }),
            )
            .await
            .unwrap();
        assert_eq!(out["file_retention_days"], Value::Null);
        assert_eq!(out["project_retention_days"], Value::Null);
        let persisted = f.state.admin.get_purge_config("proj1").await.unwrap();
        assert_eq!(persisted.file_retention_days, None);
        assert_eq!(persisted.project_retention_days, None);
    }

    /// E2E-NEW-022: the minimum valid positive value succeeds.
    #[tokio::test]
    async fn minimum_positive_retention_succeeds() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        let out = f
            .call(
                &r,
                "alice@test.com",
                "admin.set_purge_config",
                json!({
                    "project_id":"proj1",
                    "autopurge_enabled":true,
                    "use_internal_purge":true,
                    "file_retention_days":1,
                }),
            )
            .await
            .unwrap();
        assert_eq!(out["file_retention_days"], 1);
    }

    /// E2E-NEW-023: re-setting identical values is idempotent, no duplicate row.
    #[tokio::test]
    async fn resetting_identical_values_is_idempotent() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "alice@test.com").await;
        let r = registry();
        let args = json!({
            "project_id":"proj1",
            "autopurge_enabled":true,
            "use_internal_purge":true,
            "file_retention_days":7,
            "project_retention_days":30,
        });
        f.call(&r, "alice@test.com", "admin.set_purge_config", args.clone()).await.unwrap();
        f.call(&r, "alice@test.com", "admin.set_purge_config", args).await.unwrap();
        let persisted = f.state.admin.get_purge_config("proj1").await.unwrap();
        assert_eq!(persisted.file_retention_days, Some(7));
        assert_eq!(persisted.project_retention_days, Some(30));
    }

    // ── admin.list_deleted_projects (SPEC-0014 US-0010) ────────────────────

    const DAY: f64 = 86_400.0;

    /// E2E-NEW-058: platform admin sees every soft-deleted project, with the
    /// countdown computed from each project's own `deleted_at`.
    #[tokio::test]
    async fn e2e_new_058_platform_admin_sees_all_deleted_projects() {
        let f = Fixture::with_config(|c| c.safety.project_purge_grace_days = 30).await;
        f.seed_soft_deleted("proj-5", "alice@test.com", 5.0 * DAY).await;
        f.seed_soft_deleted("proj-10", "bob@test.com", 10.0 * DAY).await;
        let r = registry();

        let out = f.call(&r, ADMIN, "admin.list_deleted_projects", json!({})).await.unwrap();
        let entries = out["deleted_projects"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        let by_id: std::collections::HashMap<&str, &Value> =
            entries.iter().map(|e| (e["project_id"].as_str().unwrap(), e)).collect();
        assert_eq!(by_id["proj-5"]["owner"], "alice@test.com");
        assert_eq!(by_id["proj-5"]["days_until_permanent_removal"], 25);
        assert_eq!(by_id["proj-10"]["owner"], "bob@test.com");
        assert_eq!(by_id["proj-10"]["days_until_permanent_removal"], 20);
    }

    /// E2E-NEW-059: single-result shape, MCP-only (no REST equivalent exists).
    #[tokio::test]
    async fn e2e_new_059_single_soft_deleted_project() {
        let f = Fixture::with_config(|c| c.safety.project_purge_grace_days = 30).await;
        f.seed_soft_deleted("proj-5", "alice@test.com", 5.0 * DAY).await;
        let r = registry();

        let out = f.call(&r, ADMIN, "admin.list_deleted_projects", json!({})).await.unwrap();
        let entries = out["deleted_projects"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["project_id"], "proj-5");
        assert_eq!(entries[0]["days_until_permanent_removal"], 25);
    }

    /// E2E-NEW-060: a non-admin caller sees only their own soft-deleted project,
    /// filtered exactly as `admin.list_projects` filters, never `ERR_FORBIDDEN`.
    #[tokio::test]
    async fn e2e_new_060_membership_filtered_for_non_admin_caller() {
        let f = Fixture::with_config(|c| c.safety.project_purge_grace_days = 30).await;
        f.seed_soft_deleted("proj-mine", "alice@test.com", 5.0 * DAY).await;
        f.seed_soft_deleted("proj-other", "bob@test.com", 10.0 * DAY).await;
        let r = registry();

        let out =
            f.call(&r, "alice@test.com", "admin.list_deleted_projects", json!({})).await.unwrap();
        let entries = out["deleted_projects"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["project_id"], "proj-mine");
    }

    /// E2E-NEW-061: zero soft-deleted projects returns an empty array, no error.
    #[tokio::test]
    async fn e2e_new_061_empty_state() {
        let f = Fixture::new().await;
        let r = registry();

        let out = f.call(&r, ADMIN, "admin.list_deleted_projects", json!({})).await.unwrap();
        assert_eq!(out["deleted_projects"].as_array().unwrap().len(), 0);
    }

    /// E2E-NEW-062: grace period already elapsed yields a zero or negative
    /// countdown, never clamped to zero.
    #[tokio::test]
    async fn e2e_new_062_countdown_not_clamped_past_grace_period() {
        let f = Fixture::with_config(|c| c.safety.project_purge_grace_days = 30).await;
        f.seed_soft_deleted("proj-overdue", "alice@test.com", 35.0 * DAY).await;
        let r = registry();

        let out = f.call(&r, ADMIN, "admin.list_deleted_projects", json!({})).await.unwrap();
        let entries = out["deleted_projects"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["days_until_permanent_removal"], -5);
    }

    #[test]
    fn list_deleted_projects_schema_has_no_parameters() {
        let r = registry();
        let s = &r.resolve("admin.list_deleted_projects").unwrap().schema;
        assert_eq!(
            s.input_schema(),
            serde_json::from_str::<Value>(r#"{"type":"object","properties":{}}"#).unwrap()
        );
    }

    // ── admin.undelete_project (SPEC-0014 US-0011) ──────────────────────────

    #[test]
    fn undelete_project_schema_matches_the_contract() {
        let r = registry();
        let s = &r.resolve("admin.undelete_project").unwrap().schema;
        assert_eq!(
            s.description,
            "Clear a project's soft-delete flag, immediately restoring access (owner or platform admin)."
        );
        assert_eq!(
            s.input_schema()["properties"]["project_id"]["description"],
            "Id of the soft-deleted project to restore."
        );
        assert_eq!(s.input_schema()["required"], json!(["project_id"]));
    }

    /// E2E-NEW-063: owner undeletes, `deleted_at` cleared to NULL.
    #[tokio::test]
    async fn e2e_new_063_owner_undeletes() {
        let f = Fixture::new().await;
        f.seed_soft_deleted("proj1", "owner@test.com", 1.0).await;
        let r = registry();

        let out = f
            .call(&r, "owner@test.com", "admin.undelete_project", json!({"project_id": "proj1"}))
            .await
            .unwrap();
        assert_eq!(out, json!({"project_id": "proj1", "undeleted": true}));
        assert_eq!(f.admin_store.deleted_at_for_test("proj1").await.unwrap(), None);
    }

    /// E2E-NEW-064: platform admin (non-owner) can also undelete.
    #[tokio::test]
    async fn e2e_new_064_platform_admin_undeletes() {
        let f = Fixture::new().await;
        f.seed_soft_deleted("proj1", "owner@test.com", 1.0).await;
        let r = registry();

        f.call(&r, ADMIN, "admin.undelete_project", json!({"project_id": "proj1"})).await.unwrap();
        assert_eq!(f.admin_store.deleted_at_for_test("proj1").await.unwrap(), None);
    }

    /// E2E-NEW-065: a project that is not soft-deleted is rejected, no state change.
    #[tokio::test]
    async fn e2e_new_065_already_live_project_rejected() {
        let f = Fixture::new().await;
        f.seed_project("proj1", "owner@test.com").await;
        let r = registry();

        let e = f
            .call(&r, "owner@test.com", "admin.undelete_project", json!({"project_id": "proj1"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert_eq!(f.admin_store.deleted_at_for_test("proj1").await.unwrap(), None);
    }

    /// E2E-NEW-066: an unknown project fails `ERR_PROJECT_NOT_FOUND`, no row created.
    #[tokio::test]
    async fn e2e_new_066_unknown_project() {
        let f = Fixture::new().await;
        let r = registry();

        let e = f
            .call(&r, ADMIN, "admin.undelete_project", json!({"project_id": "ghost"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::PROJECT_NOT_FOUND);
        assert!(f.state.admin.get_project("ghost").await.unwrap().is_none());
    }

    /// E2E-NEW-067: a non-owner, non-admin member is forbidden, `deleted_at` unchanged.
    #[tokio::test]
    async fn e2e_new_067_non_owner_non_admin_forbidden() {
        let f = Fixture::new().await;
        f.seed_soft_deleted("proj1", "owner@test.com", 1.0).await;
        f.state.admin.add_member("proj1", "member@test.com", "owner@test.com").await.unwrap();
        let r = registry();

        let e = f
            .call(&r, "member@test.com", "admin.undelete_project", json!({"project_id": "proj1"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, code::FORBIDDEN);
        assert!(f.admin_store.deleted_at_for_test("proj1").await.unwrap().is_some());
    }

    /// E2E-NEW-068: undelete restores real access through the live gate
    /// (`require_member`, which `state.authorize` wraps), not a cached state.
    #[tokio::test]
    async fn e2e_new_068_undelete_restores_real_access() {
        let f = Fixture::new().await;
        f.seed_soft_deleted("proj1", "owner@test.com", 1.0).await;
        let r = registry();

        let e = f.state.authorize("proj1", "owner@test.com").await.unwrap_err();
        assert_eq!(e.code, code::PROJECT_NOT_FOUND);

        f.call(&r, "owner@test.com", "admin.undelete_project", json!({"project_id": "proj1"}))
            .await
            .unwrap();

        f.state.authorize("proj1", "owner@test.com").await.unwrap();
    }
}
