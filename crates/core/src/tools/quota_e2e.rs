//! SPEC-0020 US-0015 acceptance tests: the project usage figure and
//! `admin.get_project_quota`, driven through the real `McpServer` tool methods
//! (the dispatch path `app.rs` routes), plus the REST route over the same state.
//!
//! No limit is enforced in this story: the tests that write only prove the
//! write is accepted and the usage figure is exact.

use crate::mcp::server::{
    AppendArgs, ApplyPatchArgs, CopyArgs, DeleteArgs, EditArgs, ExportZipArgs, GetProjectQuotaArgs,
    GitCommitArgs, McpServer, MountOnlyArgs, MultiEditArgs, ReadArgs, SetProjectQuotaArgs,
    WriteArgs, WriteBytesArgs, WriteDocxArgs,
};
use crate::state::AppState;
use crate::tools::admin::test_support::Fixture;
use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::sync::Arc;

const ATLAS: &str = "Atlas";
const MIA: &str = "mia@example.com";
const OLIVIA: &str = "olivia@example.com";
const ADA: &str = "ada@example.com";
const OTTO: &str = "otto@example.com";
const MB: i64 = 1_048_576;
const SESSION_BUDGET: i64 = 262_144;

fn text_of(r: &CallToolResult) -> String {
    r.content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text content block")
}

fn ok_json(r: Result<CallToolResult, rmcp::ErrorData>) -> Value {
    let r = r.expect("a tool result, not a protocol error");
    assert_ne!(r.is_error, Some(true), "expected success, got: {}", text_of(&r));
    serde_json::from_str(&text_of(&r)).expect("the tool result is JSON")
}

fn err_text(r: Result<CallToolResult, rmcp::ErrorData>) -> String {
    let r = r.expect("a tool result, not a protocol error");
    assert_eq!(r.is_error, Some(true), "expected an error, got: {}", text_of(&r));
    text_of(&r)
}

/// Bytes that do not compress, so a zip of them is about their size.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// The shared setup: Olivia owns Atlas, Mia is a member, Ada is a platform
/// admin, Otto belongs to nothing; every session budget is 262 144 bytes.
struct World {
    f: Fixture,
}

impl World {
    async fn new(git: bool) -> World {
        let f = Fixture::with_config(|c| {
            c.auth.admins = vec![ADA.to_string()];
            c.safety.write_quota_bytes = SESSION_BUDGET;
            c.git.enabled = git;
            // The independent test purges a trashed file permanently.
            c.safety.allow_hard_delete = true;
        })
        .await;
        f.seed_project(ATLAS, OLIVIA).await;
        f.state.admin.add_member(ATLAS, MIA, OLIVIA).await.unwrap();
        World { f }
    }

    /// Atlas with a 1 MB limit using 1 000 000 bytes, the story's default.
    async fn shared() -> World {
        let w = World::new(false).await;
        w.limit(Some(MB)).await;
        w.seed("/seed.bin", 1_000_000).await;
        w
    }

    async fn limit(&self, quota_bytes: Option<i64>) {
        self.f.state.admin.set_quota(ATLAS, quota_bytes).await.unwrap();
    }

    /// Seed bytes straight into the volume: setup, not the operation tested,
    /// so no session budget is spent.
    async fn seed(&self, path: &str, len: usize) {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.write_bytes_atomic(path, &noise(len, len as u64)).await.unwrap();
    }

    fn as_(&self, person: &str) -> McpServer {
        McpServer::new(self.f.state.clone(), person.to_string())
    }

    async fn quota(&self, person: &str, project: &str) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = GetProjectQuotaArgs { project_id: project.into() };
        self.as_(person).admin_get_project_quota(Parameters(args)).await
    }

    /// Ada's reading of Atlas's usage.
    async fn used(&self) -> i64 {
        ok_json(self.quota(ADA, ATLAS).await)["used_bytes"].as_i64().expect("an integer")
    }

    /// The usage computed independently of the counter: a sum over the tree.
    async fn independent_sum(&self) -> i64 {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.meta.subtree("/").await.unwrap().iter().filter(|n| n.is_file()).map(|n| n.size).sum()
    }

    async fn write(&self, person: &str, path: &str, len: usize) -> Value {
        let args = WriteBytesArgs {
            mount_id: ATLAS.into(),
            path: path.into(),
            base64: base64::engine::general_purpose::STANDARD.encode(noise(len, 7)),
            overwrite: false,
            create_parents: true,
            trigger_documentation_service: false,
        };
        ok_json(self.as_(person).fs_write_bytes(Parameters(args)).await)
    }
}

/// SPEC-0020/E2E-001: a new 40 000 byte file is accepted and counted
/// (FR-NEW-001, FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_001_a_new_file_is_counted_in_the_usage() {
    let w = World::shared().await;
    w.write(MIA, "/a.txt", 40_000).await;
    assert_eq!(w.used().await, 1_040_000);
    assert_eq!(w.independent_sum().await, 1_040_000);
}

/// SPEC-0020/E2E-016: a copy counts the copied path again even though the
/// content is shared (FR-NEW-002).
#[tokio::test]
async fn spec_0020_e2e_016_a_copy_counts_its_path_even_with_shared_content() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 960_000).await;
    w.seed("/s.bin", 40_000).await;
    assert_eq!(w.used().await, 1_000_000);

    let args = CopyArgs {
        mount_id: ATLAS.into(),
        source: "/s.bin".into(),
        destination: "/s2.bin".into(),
        overwrite: false,
        recursive: false,
    };
    ok_json(w.as_(MIA).fs_copy(Parameters(args)).await);
    assert_eq!(w.used().await, 1_040_000);
}

/// SPEC-0020/E2E-020: an export zip is not project storage (FR-NEW-002).
#[tokio::test]
async fn spec_0020_e2e_020_an_export_leaves_the_usage_unchanged() {
    let w = World::shared().await;
    let args = ExportZipArgs { mount_id: ATLAS.into(), paths: vec!["/".into()] };
    let out = ok_json(w.as_(MIA).fs_export_zip(Parameters(args)).await);
    assert!(out["url"].as_str().is_some_and(|u| !u.is_empty()), "got: {out}");
    assert_eq!(w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-021: version control history is not project storage
/// (FR-NEW-002).
#[tokio::test]
async fn spec_0020_e2e_021_a_commit_leaves_the_usage_unchanged() {
    let w = World::new(true).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 1_000_000).await;
    let store = Arc::new(crate::git::GitRepoStore::new(
        w.f.state.config.clone(),
        w.f.state.stores.relational().clone(),
    ));

    let ctx = w.f.ctx(MIA);
    let init = MountOnlyArgs { mount_id: ATLAS.into() };
    crate::tools::git::tool_init(ctx.clone(), init, Some(store.clone())).await.unwrap();
    let commit = GitCommitArgs {
        mount_id: ATLAS.into(),
        message: "everything".into(),
        author_name: None,
        author_email: None,
    };
    let out = crate::tools::git::tool_commit(ctx, commit, Some(store)).await.unwrap();
    assert!(out["commit_sha"].as_str().is_some(), "got: {out}");
    assert_eq!(w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-037: a platform admin reads the limit and the usage
/// (FR-NEW-009).
#[tokio::test]
async fn spec_0020_e2e_037_the_admin_reads_limit_and_usage() {
    let w = World::shared().await;
    assert_eq!(
        ok_json(w.quota(ADA, ATLAS).await),
        json!({"project_id": "Atlas", "max_mb": 1, "used_bytes": 1_000_000})
    );
}

/// SPEC-0020/E2E-038: the owner reads the same figures (FR-NEW-009).
#[tokio::test]
async fn spec_0020_e2e_038_the_owner_reads_limit_and_usage() {
    let w = World::shared().await;
    assert_eq!(
        ok_json(w.quota(OLIVIA, ATLAS).await),
        json!({"project_id": "Atlas", "max_mb": 1, "used_bytes": 1_000_000})
    );
}

/// SPEC-0020/E2E-039: a member who is not the owner is refused, and the
/// refusal shows no figure (FR-NEW-010).
#[tokio::test]
async fn spec_0020_e2e_039_a_plain_member_is_refused() {
    let w = World::shared().await;
    let err = err_text(w.quota(MIA, ATLAS).await);
    assert!(
        err.ends_with("ERR_FORBIDDEN: 'mia@example.com' is not the owner of 'Atlas'"),
        "got: {err}"
    );
    assert!(!err.contains("1000000") && !err.contains("max_mb"), "got: {err}");
}

/// SPEC-0020/E2E-040: a person outside the project is refused (FR-NEW-010).
#[tokio::test]
async fn spec_0020_e2e_040_an_outsider_is_refused() {
    let w = World::shared().await;
    let err = err_text(w.quota(OTTO, ATLAS).await);
    assert!(
        err.ends_with("ERR_FORBIDDEN: 'otto@example.com' is not the owner of 'Atlas'"),
        "got: {err}"
    );
}

/// SPEC-0020/E2E-041: a missing project reads as not owned for anyone but an
/// admin, so its existence cannot be probed (FR-NEW-010).
#[tokio::test]
async fn spec_0020_e2e_041_a_missing_project_is_only_revealed_to_an_admin() {
    let w = World::shared().await;
    let otto = err_text(w.quota(OTTO, "Zephyr").await);
    assert!(
        otto.ends_with("ERR_FORBIDDEN: 'otto@example.com' is not the owner of 'Zephyr'"),
        "got: {otto}"
    );
    let ada = err_text(w.quota(ADA, "Zephyr").await);
    assert!(ada.ends_with("ERR_PROJECT_NOT_FOUND: project 'Zephyr' not found"), "got: {ada}");
}

/// SPEC-0020/E2E-042: no limit reads as a JSON null, never 0 or text
/// (FR-NEW-009, FR-NEW-011).
#[tokio::test]
async fn spec_0020_e2e_042_no_limit_reads_as_null() {
    let w = World::new(false).await;
    w.seed("/big.bin", 5_000_000).await;
    let out = ok_json(w.quota(ADA, ATLAS).await);
    assert!(out["max_mb"].is_null(), "got: {out}");
    assert_eq!(out, json!({"project_id": "Atlas", "max_mb": null, "used_bytes": 5_000_000}));
}

/// SPEC-0020/E2E-043: usage is computed at the moment of the read
/// (FR-NEW-009).
#[tokio::test]
async fn spec_0020_e2e_043_usage_is_read_at_the_moment_asked() {
    let w = World::new(false).await;
    assert_eq!(w.used().await, 0);
    w.write(MIA, "/n.txt", 1_234).await;
    assert_eq!(w.used().await, 1_234);
}

/// SPEC-0020/E2E-053: without a limit a write is refused for nothing but the
/// session budget, and is counted (FR-NEW-011).
#[tokio::test]
async fn spec_0020_e2e_053_no_limit_refuses_nothing() {
    let w = World::new(false).await;
    w.seed("/big.bin", 5_000_000).await;
    w.write(MIA, "/w.bin", SESSION_BUDGET as usize).await;
    assert_eq!(w.used().await, 5_262_144);
}

/// The story's independent test: trashed files are counted, a permanent delete
/// is not, and the counter agrees with a sum read from the store (FR-NEW-002).
#[tokio::test]
async fn spec_0020_us_0015_trash_counts_and_purge_releases() {
    let w = World::new(false).await;
    w.write(MIA, "/one.bin", 1_000).await;
    w.write(MIA, "/two.bin", 2_000).await;
    w.write(MIA, "/three.bin", 3_000).await;

    let trash = DeleteArgs {
        mount_id: ATLAS.into(),
        path: "/two.bin".into(),
        recursive: false,
        trash: true,
    };
    ok_json(w.as_(MIA).fs_delete(Parameters(trash)).await);
    assert_eq!(w.used().await, 6_000, "a trashed file still counts");

    let client = w.f.state.stores.client(ATLAS).await.unwrap();
    let trashed = client
        .meta
        .subtree("/")
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.is_file() && n.path != "/one.bin" && n.path != "/three.bin")
        .expect("the trashed file")
        .path;
    assert_ne!(trashed, "/two.bin");
    let purge =
        DeleteArgs { mount_id: ATLAS.into(), path: trashed, recursive: false, trash: false };
    ok_json(w.as_(MIA).fs_delete(Parameters(purge)).await);

    assert_eq!(w.used().await, 4_000);
    assert_eq!(w.independent_sum().await, 4_000);
}

/// The REST route answers the MCP tool's bytes for a success and its codes
/// for both refusals (SPEC-0020 DEC-006, DEC-007).
#[tokio::test]
async fn spec_0020_rest_quota_route_matches_the_tool() {
    use tower::ServiceExt as _;

    let w = World::shared().await;
    let keys = tempfile::tempdir().unwrap();
    let (key, public) = crate::keys::write_keypair(keys.path()).unwrap();
    let mut config = (*w.f.state.config).clone();
    config.auth.jwt.public_key_path = public.display().to_string();
    let s = &w.f.state;
    let state = AppState {
        config: Arc::new(config.clone()),
        admin: s.admin.clone(),
        stores: s.stores.clone(),
        safety: s.safety.clone(),
        identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
        editors: s.editors.clone(),
        doc_service: None,
        search: None,
    };
    let app = crate::api::dataplane::router(Arc::new(state));
    let get = |person: &str, project: &str| {
        let token = crate::keys::mint_token_from_file(
            &key,
            person,
            crate::keys::DEFAULT_ISSUER,
            crate::keys::DEFAULT_CLAIM,
            3600,
        )
        .unwrap();
        let req = axum::http::Request::builder()
            .uri(format!("/api/admin/projects/{project}/quota"))
            .header("Authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let app = app.clone();
        async move {
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status().as_u16();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8(bytes.to_vec()).unwrap())
        }
    };

    let (status, body) = get(OLIVIA, ATLAS).await;
    assert_eq!(status, 200, "{body}");
    let r = w.quota(OLIVIA, ATLAS).await.unwrap();
    assert_eq!(body, text_of(&r), "the REST body is the tool's, byte for byte");

    let (status, body) = get(MIA, ATLAS).await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("'mia@example.com' is not the owner of 'Atlas'"), "{body}");

    let (status, body) = get(OTTO, "Zephyr").await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = get(ADA, "Zephyr").await;
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("project 'Zephyr' not found"), "{body}");
}

// ── US-0016: single file writes refused over the limit ──────────────────────

/// The refusal text of FR-NEW-001, code included, for a 1 MB limit.
fn refusal(used: i64, requested: i64) -> String {
    format!(
        "ERR_PROJECT_QUOTA_EXCEEDED: project storage quota of 1 MB exceeded: \
         {used} bytes used, {requested} bytes requested"
    )
}

const SESSION_REFUSAL: &str =
    "ERR_WRITE_QUOTA_EXCEEDED: session write quota of 262144 bytes exceeded";

/// Printable text of `len` bytes, so the edit tools can work on it.
fn text(len: usize, seed: u64) -> String {
    noise(len, seed).into_iter().map(|b| char::from(b'a' + b % 26)).collect()
}

impl World {
    /// Seed a text file straight into the volume (setup, no session budget).
    async fn seed_text(&self, path: &str, content: &str) {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.write_bytes_atomic(path, content.as_bytes()).await.unwrap();
    }

    async fn content(&self, path: &str) -> Option<Vec<u8>> {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.meta.get(path).await.unwrap()?;
        Some(client.read_bytes(path).await.unwrap())
    }

    async fn try_write(&self, person: &str, path: &str, len: usize) -> RawResult {
        let args = WriteBytesArgs {
            mount_id: ATLAS.into(),
            path: path.into(),
            base64: base64::engine::general_purpose::STANDARD.encode(noise(len, 11)),
            overwrite: false,
            create_parents: true,
            trigger_documentation_service: false,
        };
        self.as_(person).fs_write_bytes(Parameters(args)).await
    }

    async fn try_write_text(&self, path: &str, content: &str, overwrite: bool) -> RawResult {
        let args = WriteArgs {
            mount_id: ATLAS.into(),
            path: path.into(),
            content: content.into(),
            overwrite,
            create_parents: true,
        };
        self.as_(MIA).fs_write(Parameters(args)).await
    }

    /// Mia reads `path`, which the read before write guard asks of an edit.
    async fn read_as_mia(&self, path: &str) {
        let args = ReadArgs {
            mount_id: ATLAS.into(),
            path: path.into(),
            offset_lines: 0,
            limit_lines: 1,
            line_numbered: false,
        };
        let r = self.as_(MIA).fs_read(Parameters(args)).await.unwrap();
        assert_ne!(r.is_error, Some(true), "read failed: {}", text_of(&r));
    }

    async fn remove_limit(&self) {
        let args = SetProjectQuotaArgs { project_id: ATLAS.into(), max_mb: None };
        ok_json(self.as_(ADA).admin_set_project_quota(Parameters(args)).await);
    }

    /// Atlas at exactly its limit: 1 048 576 bytes, `/r.md` 100 000 of them.
    async fn full() -> World {
        let w = World::new(false).await;
        w.limit(Some(MB)).await;
        w.seed("/seed.bin", (MB - 100_000) as usize).await;
        w.seed_text("/r.md", &text(100_000, 3)).await;
        assert_eq!(w.used().await, MB);
        w
    }
}

type RawResult = Result<CallToolResult, rmcp::ErrorData>;

/// SPEC-0020/E2E-002: a write over the limit is refused with the exact
/// message, and changes nothing (FR-NEW-001, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_002_a_write_over_the_limit_is_refused() {
    let w = World::shared().await;
    let err = err_text(w.try_write(MIA, "/b.txt", 50_000).await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    assert!(w.content("/b.txt").await.is_none(), "no node at the refused path");
    assert_eq!(w.used().await, 1_000_000);
    assert_eq!(w.independent_sum().await, 1_000_000);
}

/// SPEC-0020/E2E-003: a write reaching exactly the limit is accepted
/// (FR-NEW-005).
#[tokio::test]
async fn spec_0020_e2e_003_a_write_to_exactly_the_limit_is_accepted() {
    let w = World::shared().await;
    w.write(MIA, "/b.txt", 48_576).await;
    assert_eq!(w.used().await, MB);
}

/// SPEC-0020/E2E-004: at the limit, one more byte is refused (FR-NEW-005,
/// FR-NEW-001).
#[tokio::test]
async fn spec_0020_e2e_004_one_byte_over_the_limit_is_refused() {
    let w = World::full().await;
    let err = err_text(w.try_write(MIA, "/one.txt", 1).await);
    assert!(err.ends_with(&refusal(MB, 1)), "got: {err}");
    assert_eq!(w.used().await, MB);
}

/// SPEC-0020/E2E-005: the project check runs before the session budget
/// (FR-NEW-006, FR-NEW-001).
#[tokio::test]
async fn spec_0020_e2e_005_the_project_limit_is_checked_first() {
    let w = World::shared().await;
    let err = err_text(w.try_write(MIA, "/big.txt", 300_000).await);
    assert!(err.ends_with(&refusal(1_000_000, 300_000)), "got: {err}");
    assert!(!err.contains("session write quota"), "got: {err}");
}

/// SPEC-0020/E2E-006: a refused write charges nothing to the session budget
/// (FR-NEW-006, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_006_a_refusal_charges_no_session_budget() {
    let w = World::shared().await;
    let err = err_text(w.try_write(MIA, "/b.txt", 50_000).await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    w.remove_limit().await;
    w.write(MIA, "/full.bin", SESSION_BUDGET as usize).await;
    let err = err_text(w.try_write(MIA, "/one.bin", 1).await);
    assert!(err.ends_with(SESSION_REFUSAL), "got: {err}");
}

/// SPEC-0020/E2E-007: within the project limit, the session budget still
/// refuses (FR-NEW-006).
#[tokio::test]
async fn spec_0020_e2e_007_the_session_budget_still_applies_under_the_limit() {
    let w = World::new(false).await;
    w.limit(Some(2 * MB)).await;
    let err = err_text(w.try_write(MIA, "/s.bin", SESSION_BUDGET as usize + 1).await);
    assert!(err.ends_with(SESSION_REFUSAL), "got: {err}");
    assert!(w.content("/s.bin").await.is_none());
}

/// SPEC-0020/E2E-008: an append is checked for the bytes it adds
/// (FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_008_an_append_over_the_limit_is_refused() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 990_000).await;
    w.seed_text("/n.md", &text(10_000, 5)).await;
    let args = AppendArgs {
        mount_id: ATLAS.into(),
        path: "/n.md".into(),
        content: text(48_577, 6),
        create: false,
    };
    let err = err_text(w.as_(MIA).fs_append(Parameters(args)).await);
    assert!(err.ends_with(&refusal(1_000_000, 48_577)), "got: {err}");
    assert_eq!(w.content("/n.md").await.unwrap().len(), 10_000);
}

/// SPEC-0020/E2E-009: an append to exactly the limit is accepted
/// (FR-NEW-003, FR-NEW-005).
#[tokio::test]
async fn spec_0020_e2e_009_an_append_to_exactly_the_limit_is_accepted() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 990_000).await;
    w.seed_text("/n.md", &text(10_000, 5)).await;
    let args = AppendArgs {
        mount_id: ATLAS.into(),
        path: "/n.md".into(),
        content: text(48_576, 6),
        create: false,
    };
    ok_json(w.as_(MIA).fs_append(Parameters(args)).await);
    assert_eq!(w.content("/n.md").await.unwrap().len(), 58_576);
    assert_eq!(w.used().await, MB);
}

/// SPEC-0020/E2E-010: an overwrite is checked for its growth only, and a
/// refusal keeps the previous content (FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_010_an_overwrite_is_checked_for_its_growth() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 900_000).await;
    let before = text(100_000, 3);
    w.seed_text("/r.md", &before).await;
    w.read_as_mia("/r.md").await;
    let err = err_text(w.try_write_text("/r.md", &text(148_577, 4), true).await);
    assert!(err.ends_with(&refusal(1_000_000, 48_577)), "got: {err}");
    assert_eq!(w.content("/r.md").await.unwrap(), before.as_bytes());
}

/// SPEC-0020/E2E-011: a same size overwrite at the limit is accepted
/// (FR-NEW-004).
#[tokio::test]
async fn spec_0020_e2e_011_a_same_size_overwrite_at_the_limit_is_accepted() {
    let w = World::full().await;
    w.read_as_mia("/r.md").await;
    ok_json(w.try_write_text("/r.md", &text(100_000, 9), true).await);
    assert_eq!(w.used().await, MB);
}

/// SPEC-0020/E2E-012: a shrinking edit at the limit is accepted
/// (FR-NEW-004).
#[tokio::test]
async fn spec_0020_e2e_012_a_shrinking_edit_at_the_limit_is_accepted() {
    let w = World::full().await;
    w.read_as_mia("/r.md").await;
    let old = String::from_utf8(w.content("/r.md").await.unwrap()).unwrap();
    let args = EditArgs {
        mount_id: ATLAS.into(),
        path: "/r.md".into(),
        // 5 000 bytes from the middle: noise text this long occurs once.
        old_string: old[40_000..45_000].to_string(),
        new_string: "z".repeat(1_000),
        replace_all: false,
        dry_run: false,
    };
    ok_json(w.as_(MIA).fs_edit(Parameters(args)).await);
    assert_eq!(w.used().await, 1_044_576);
}

/// SPEC-0020/E2E-013: a growing edit over the limit is refused and changes
/// nothing (FR-NEW-003, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_013_a_growing_edit_over_the_limit_is_refused() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 999_999).await;
    w.seed_text("/r.md", "X").await;
    w.read_as_mia("/r.md").await;
    let args = EditArgs {
        mount_id: ATLAS.into(),
        path: "/r.md".into(),
        old_string: "X".into(),
        new_string: text(48_578, 8),
        replace_all: false,
        dry_run: false,
    };
    let err = err_text(w.as_(MIA).fs_edit(Parameters(args)).await);
    assert!(err.ends_with(&refusal(1_000_000, 48_577)), "got: {err}");
    assert_eq!(w.content("/r.md").await.unwrap(), b"X");
}

/// SPEC-0020/E2E-014, driven through the REST upload route (the multipart
/// `POST /api/fs/{mount_id}/upload`): an upload over the limit is refused
/// with HTTP 507 and `ERR_PROJECT_QUOTA_EXCEEDED`, and stores nothing
/// (FR-NEW-003, DEC-005).
#[tokio::test]
async fn spec_0020_e2e_014_an_upload_over_the_limit_is_refused_with_507() {
    use tower::ServiceExt as _;

    let w = World::shared().await;
    let (app, token) = rest(&w, MIA);
    let boundary = "quotaboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&noise(60_000, 13));
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/fs/{ATLAS}/upload"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, 507, "{body}");
    assert_eq!(body["error"], "ERR_PROJECT_QUOTA_EXCEEDED", "{body}");
    assert_eq!(body["detail"], refusal(1_000_000, 60_000), "{body}");
    assert!(w.content("/photo.png").await.is_none());
    assert_eq!(w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-015: a file copy is checked for the copied size
/// (FR-NEW-002, FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_015_a_copy_over_the_limit_is_refused() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 950_000).await;
    w.seed("/big.bin", 50_000).await;
    let args = CopyArgs {
        mount_id: ATLAS.into(),
        source: "/big.bin".into(),
        destination: "/big2.bin".into(),
        overwrite: false,
        recursive: false,
    };
    let err = err_text(w.as_(MIA).fs_copy(Parameters(args)).await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    assert!(w.content("/big2.bin").await.is_none());
}

/// SPEC-0020/E2E-017: a Word document is checked for its rendered size
/// (FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_017_a_word_document_over_the_limit_is_refused() {
    let w = World::shared().await;
    // Spaced noise words, so the rendered document does not compress away.
    let markdown: String =
        (0..30).map(|i| format!("Paragraph {i} {}\n\n", text(6_000, i))).collect();
    let docx = |mount: &str| WriteDocxArgs {
        mount_id: mount.into(),
        path: "/w.docx".into(),
        markdown: markdown.clone(),
        title: None,
        overwrite: false,
    };

    // N: the size the same content renders to in a project with no limit.
    w.f.seed_project("Zephyr", OLIVIA).await;
    let free = ok_json(w.as_(OLIVIA).fs_write_docx(Parameters(docx("Zephyr"))).await);
    let n = free["bytes_written"].as_i64().unwrap();
    assert!(n > 48_576 && n < SESSION_BUDGET, "N = {n}");

    let err = err_text(w.as_(MIA).fs_write_docx(Parameters(docx(ATLAS))).await);
    assert!(err.ends_with(&refusal(1_000_000, n)), "got: {err}");
    assert!(w.content("/w.docx").await.is_none());
}

/// SPEC-0020/E2E-054: without a limit only the session budget refuses
/// (FR-NEW-011, FR-NEW-006).
#[tokio::test]
async fn spec_0020_e2e_054_no_limit_leaves_only_the_session_budget() {
    let w = World::new(false).await;
    let err = err_text(w.try_write(MIA, "/s.bin", SESSION_BUDGET as usize + 1).await);
    assert!(err.ends_with(SESSION_REFUSAL), "got: {err}");
    assert!(!err.contains("project storage quota"), "got: {err}");
}

/// SPEC-0020/E2E-055: once the limit is removed the refused write goes
/// through (FR-NEW-011).
#[tokio::test]
async fn spec_0020_e2e_055_removing_the_limit_lets_the_retry_through() {
    let w = World::shared().await;
    let err = err_text(w.try_write(MIA, "/b.txt", 50_000).await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    w.remove_limit().await;
    ok_json(w.try_write(MIA, "/b.txt", 50_000).await);
    assert_eq!(w.used().await, 1_050_000);
}

/// SPEC-0020/E2E-056: a write larger than both the limit and the session
/// budget names the project limit (FR-NEW-005, FR-NEW-006).
#[tokio::test]
async fn spec_0020_e2e_056_an_oversized_write_on_an_empty_project_names_the_limit() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    let err = err_text(w.try_write(MIA, "/huge.bin", MB as usize + 1).await);
    assert!(err.ends_with(&refusal(0, MB + 1)), "got: {err}");
    assert!(!err.contains("session write quota"), "got: {err}");
}

/// SPEC-0020/E2E-057: an empty file at the limit is accepted (FR-NEW-004,
/// FR-NEW-005).
#[tokio::test]
async fn spec_0020_e2e_057_an_empty_file_at_the_limit_is_accepted() {
    let w = World::full().await;
    ok_json(w.try_write_text("/empty.txt", "", false).await);
    assert_eq!(w.used().await, MB);
    assert_eq!(w.content("/empty.txt").await.unwrap(), b"");
}

/// A patch adding a file over the limit is refused with the exact message and
/// writes nothing (FR-NEW-001, FR-NEW-003, FR-NEW-013).
#[tokio::test]
async fn spec_0020_us_0016_an_apply_patch_add_over_the_limit_is_refused() {
    let w = World::shared().await;
    let patch =
        format!("*** Begin Patch\n*** Add File: /p.md\n+{}\n*** End Patch", text(50_000, 21));
    let args = ApplyPatchArgs { mount_id: ATLAS.into(), patch_text: patch };
    let err = err_text(w.as_(MIA).fs_apply_patch(Parameters(args)).await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    assert!(w.content("/p.md").await.is_none(), "no node at the refused path");
    assert_eq!(w.used().await, 1_000_000);
    assert_eq!(w.independent_sum().await, 1_000_000);
}

/// A growing multi edit over the limit is refused and changes nothing
/// (FR-NEW-003, FR-NEW-013).
#[tokio::test]
async fn spec_0020_us_0016_a_multi_edit_over_the_limit_is_refused() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 999_999).await;
    w.seed_text("/r.md", "X").await;
    w.read_as_mia("/r.md").await;
    let args = MultiEditArgs {
        mount_id: ATLAS.into(),
        path: "/r.md".into(),
        edits: json!([{"old_string": "X", "new_string": text(48_578, 8)}]),
        dry_run: false,
    };
    let err = err_text(w.as_(MIA).fs_multi_edit(Parameters(args)).await);
    assert!(err.ends_with(&refusal(1_000_000, 48_577)), "got: {err}");
    assert_eq!(w.content("/r.md").await.unwrap(), b"X");
    assert_eq!(w.used().await, 1_000_000);
}

/// The REST data plane over the world's state, with a bearer token for
/// `person`.
fn rest(w: &World, person: &str) -> (axum::Router, String) {
    let keys = tempfile::tempdir().unwrap();
    let (key, public) = crate::keys::write_keypair(keys.path()).unwrap();
    let mut config = (*w.f.state.config).clone();
    config.auth.jwt.public_key_path = public.display().to_string();
    let s = &w.f.state;
    let state = AppState {
        config: Arc::new(config.clone()),
        admin: s.admin.clone(),
        stores: s.stores.clone(),
        safety: s.safety.clone(),
        identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
        editors: s.editors.clone(),
        doc_service: None,
        search: None,
    };
    let token = crate::keys::mint_token_from_file(
        &key,
        person,
        crate::keys::DEFAULT_ISSUER,
        crate::keys::DEFAULT_CLAIM,
        3600,
    )
    .unwrap();
    (crate::api::dataplane::router(Arc::new(state)), token)
}
