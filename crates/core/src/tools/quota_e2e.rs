//! SPEC-0020 US-0015 acceptance tests: the project usage figure and
//! `admin.get_project_quota`, driven through the real `McpServer` tool methods
//! (the dispatch path `app.rs` routes), plus the REST route over the same state.
//!
//! No limit is enforced in this story: the tests that write only prove the
//! write is accepted and the usage figure is exact.

use crate::docs::DocService;
use crate::docs::service::{Conversion, Manifest, PictureMeta, StubDocService};
use crate::mcp::server::{
    AppendArgs, ApplyPatchArgs, CopyArgs, DeleteArgs, DocumentizeArgs, EditArgs, ExportZipArgs,
    ExtractArchiveArgs, ExtractTextArgs, GetProjectQuotaArgs, GitCommitArgs, ListImagesArgs,
    ListTablesArgs, McpServer, MountOnlyArgs, MultiEditArgs, ReadArgs, SetProjectQuotaArgs,
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

    /// [`Self::as_`] with `doc` as the document service (SPEC-0020 US-0017),
    /// sharing the fixture's stores and session accounting, so a conversion's
    /// sibling and the usage read see the same state.
    fn as_with_doc(&self, person: &str, doc: Option<Arc<dyn DocService>>) -> McpServer {
        let s = &self.f.state;
        let state = AppState {
            config: s.config.clone(),
            admin: s.admin.clone(),
            stores: s.stores.clone(),
            safety: s.safety.clone(),
            identity: s.identity.clone(),
            editors: s.editors.clone(),
            doc_service: doc,
            search: None,
        };
        McpServer::new(Arc::new(state), person.to_string())
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

    /// Write real text straight into the volume: setup, not the operation
    /// tested, so no session budget is spent (mirrors [`Self::seed`]).
    async fn put_text(&self, path: &str, content: &str) {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.write_text_atomic(path, content).await.unwrap();
    }

    async fn list_images(&self, person: &str, path: &str) -> Value {
        let args = ListImagesArgs { mount_id: ATLAS.into(), path: path.into(), marker: None };
        ok_json(self.as_(person).fs_list_images(Parameters(args)).await)
    }

    async fn list_tables(&self, person: &str, path: &str) -> Value {
        let args = ListTablesArgs { mount_id: ATLAS.into(), path: path.into(), marker: None };
        ok_json(self.as_(person).fs_list_tables(Parameters(args)).await)
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

// ── US-0017: conversion Markdown under the limit ────────────────────────────

/// The refusal message of FR-NEW-001 alone, the shape a documentation outcome
/// or `fs.documentize` carries (no `ERR_*` prefix: that lives in `code`).
fn refusal_message(used: i64, requested: i64) -> String {
    format!(
        "project storage quota of 1 MB exceeded: {used} bytes used, {requested} bytes requested"
    )
}

/// A conversion's Markdown of exactly `len` bytes, no image or table: the
/// artifact set it captures is empty, so its only weight is its own text.
fn plain_markdown(len: usize) -> String {
    "x".repeat(len)
}

/// A Markdown of exactly `total_len` bytes holding 4 picture references and 2
/// pipe tables, plus the 4 pictures themselves, the 4 images totalling
/// exactly 120 000 bytes (SPEC-0020/E2E-019): the pictures and tables are
/// session weight only (DEC-008), never counted against the project.
fn markdown_with_pictures(total_len: usize) -> (String, Vec<(String, Vec<u8>, String)>) {
    let mut md = String::new();
    let mut pics = Vec::new();
    for n in 1..=4 {
        md.push_str(&format!("![fig{n}](figures/f{n}.png)\n\n"));
        pics.push((format!("figures/f{n}.png"), noise(30_000, n as u64), format!("Figure {n}")));
    }
    md.push_str("| Region | Jan |\n| --- | --- |\n| North | 10 |\n\n");
    md.push_str("| Item | EUR |\n| --- | --- |\n| Rent | 900 |\n\n");
    let pad = total_len.saturating_sub(md.len());
    md.push_str(&"x".repeat(pad));
    (md, pics)
}

/// The fake converter's bundle for `markdown`, `pics` keyed `(target, bytes,
/// caption)`.
fn bundle_conversion(markdown: String, pics: Vec<(String, Vec<u8>, String)>) -> Conversion {
    let files = pics.iter().map(|(t, b, _)| (t.clone(), b.clone())).collect();
    let pictures = pics
        .into_iter()
        .map(|(t, _, caption)| (t, PictureMeta { caption, caption_failed: false, page: None }))
        .collect();
    Conversion { markdown, files, manifest: Some(Manifest { pictures }) }
}

/// Mia uploads `len` bytes to `path` requesting conversion through `doc`.
async fn upload_pdf(
    w: &World,
    person: &str,
    path: &str,
    len: usize,
    doc: Arc<dyn DocService>,
) -> RawResult {
    let args = WriteBytesArgs {
        mount_id: ATLAS.into(),
        path: path.into(),
        base64: base64::engine::general_purpose::STANDARD.encode(noise(len, 99)),
        overwrite: false,
        create_parents: true,
        trigger_documentation_service: true,
    };
    w.as_with_doc(person, Some(doc)).fs_write_bytes(Parameters(args)).await
}

/// SPEC-0020/E2E-018: an upload whose conversion sibling would exceed the
/// limit is accepted, keeps the source, writes no Markdown, and reports the
/// refusal as the documentation outcome (FR-NEW-014, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_018_a_refused_sibling_keeps_the_upload_and_reports_the_refusal() {
    let w = World::shared().await;
    let doc: Arc<dyn DocService> = Arc::new(StubDocService::ok(plain_markdown(20_000)));
    let out = ok_json(upload_pdf(&w, MIA, "/d.pdf", 30_000, doc).await);
    assert_eq!(
        out["documentation"]["error"]["message"],
        json!(refusal_message(1_030_000, 20_000)),
        "got: {out}"
    );
    assert!(w.content("/d.pdf").await.is_some(), "the upload is kept");
    assert!(w.content("/d.md").await.is_none(), "no markdown sibling");
    assert_eq!(w.used().await, 1_030_000);
}

/// SPEC-0020/E2E-019: a conversion's pictures and tables stay out of the
/// project delta, only the Markdown sibling counts (FR-NEW-002).
#[tokio::test]
async fn spec_0020_e2e_019_artifacts_are_not_counted_against_the_project() {
    let w = World::shared().await;
    let (markdown, pics) = markdown_with_pictures(5_000);
    let doc: Arc<dyn DocService> =
        Arc::new(StubDocService::bundle(bundle_conversion(markdown, pics)));
    let out = ok_json(upload_pdf(&w, MIA, "/e.pdf", 10_000, doc).await);
    assert_eq!(out["documentation"]["md_path"], json!("/e.md"), "got: {out}");
    assert_eq!(w.used().await, 1_015_000);
    // The 4 images and 2 tables are kept as artifacts, out of the project
    // delta: 120 000 bytes of pictures plus 2 tables that never land in
    // usage (DEC-008).
    let images = w.list_images(MIA, "/e.pdf").await;
    assert_eq!(images["images"].as_array().unwrap().len(), 4, "got: {images}");
    let tables = w.list_tables(MIA, "/e.pdf").await;
    assert_eq!(tables["tables"].as_array().unwrap().len(), 2, "got: {tables}");
}

/// SPEC-0020/E2E-058: an upload and its Markdown sibling, both under the
/// limit, are both accepted (FR-NEW-014).
#[tokio::test]
async fn spec_0020_e2e_058_an_upload_and_its_sibling_are_both_accepted() {
    let w = World::shared().await;
    let doc: Arc<dyn DocService> = Arc::new(StubDocService::ok(plain_markdown(5_000)));
    let out = ok_json(upload_pdf(&w, MIA, "/f.pdf", 10_000, doc).await);
    assert_eq!(out["documentation"]["md_path"], json!("/f.md"), "got: {out}");
    assert_eq!(w.used().await, 1_015_000);
}

/// SPEC-0020/E2E-059: the source itself is refused before any conversion is
/// attempted (FR-NEW-014, FR-NEW-001).
#[tokio::test]
async fn spec_0020_e2e_059_an_upload_over_the_limit_on_its_own_bytes_is_refused() {
    let w = World::shared().await;
    let doc: Arc<dyn DocService> = Arc::new(StubDocService::ok(plain_markdown(1)));
    let err = err_text(upload_pdf(&w, MIA, "/g.pdf", 50_000, doc).await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    assert!(w.content("/g.pdf").await.is_none());
    assert!(w.content("/g.md").await.is_none());
    assert_eq!(w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-060: the upload and its sibling may together land exactly at
/// the limit (FR-NEW-014, FR-NEW-005).
#[tokio::test]
async fn spec_0020_e2e_060_the_upload_and_its_sibling_may_land_exactly_at_the_limit() {
    let w = World::shared().await;
    let doc: Arc<dyn DocService> = Arc::new(StubDocService::ok(plain_markdown(8_576)));
    let out = ok_json(upload_pdf(&w, MIA, "/h.pdf", 40_000, doc).await);
    assert_eq!(out["documentation"]["md_path"], json!("/h.md"), "got: {out}");
    assert_eq!(w.used().await, MB);
}

/// `fs.documentize` applies the same project check as the upload path, the
/// refusal reported as the tool's own error (no source write to protect). A
/// re-conversion over the limit, with an existing sibling and artifact set
/// already in place, leaves both exactly as they were (FR-NEW-014,
/// FR-NEW-002).
#[tokio::test]
async fn spec_0020_documentize_over_the_limit_is_refused_as_the_tool_error() {
    let w = World::shared().await;
    w.seed("/doc.pdf", 20_000).await;
    assert_eq!(w.used().await, 1_020_000);

    // A first, accepted conversion leaves a sibling and an artifact set in place.
    let (first_markdown, first_pics) = markdown_with_pictures(5_000);
    let first_doc: Arc<dyn DocService> =
        Arc::new(StubDocService::bundle(bundle_conversion(first_markdown, first_pics)));
    let args =
        DocumentizeArgs { mount_id: ATLAS.into(), path: "/doc.pdf".into(), overwrite: false };
    let out = ok_json(w.as_with_doc(MIA, Some(first_doc)).fs_documentize(Parameters(args)).await);
    assert_eq!(out["md_path"], json!("/doc.md"), "got: {out}");
    assert_eq!(w.used().await, 1_025_000);
    let sibling_before = w.content("/doc.md").await.expect("the first sibling");
    let images_before = w.list_images(MIA, "/doc.pdf").await;
    let tables_before = w.list_tables(MIA, "/doc.pdf").await;

    let doc: Arc<dyn DocService> = Arc::new(StubDocService::ok(plain_markdown(40_000)));
    let args = DocumentizeArgs { mount_id: ATLAS.into(), path: "/doc.pdf".into(), overwrite: true };
    let err = err_text(w.as_with_doc(MIA, Some(doc)).fs_documentize(Parameters(args)).await);
    // The delta is checked against the sibling's own growth, 40 000 minus
    // the 5 000 byte sibling already there (SPEC-0020 FR-NEW-014).
    assert_eq!(
        err,
        format!("An error occurred invoking 'fs.documentize': {}", refusal(1_025_000, 35_000)),
        "got: {err}"
    );

    assert_eq!(w.content("/doc.md").await, Some(sibling_before), "sibling bytes unchanged");
    assert_eq!(w.list_images(MIA, "/doc.pdf").await, images_before, "images unchanged");
    assert_eq!(w.list_tables(MIA, "/doc.pdf").await, tables_before, "tables unchanged");
    assert_eq!(w.used().await, 1_025_000);
}

/// A conversion's Markdown table of one column, `header` then `cell`
/// (SPEC-0020): the exact byte shape `fs.extract_text`'s CSV path renders,
/// so the test can predict the sibling's size without reading it back.
fn one_column_table_md(header: &str, cell: &str) -> String {
    format!("| {header} |\n| --- |\n| {cell} |")
}

/// SPEC-0020: `fs.extract_text` on a document whose Markdown sibling would
/// exceed the limit is refused with the exact project quota message,
/// computed against the sibling's own growth over what was there before
/// (FR-NEW-014), even though the source rewrite that triggers the refresh
/// fits under the limit on its own. The previous sibling and its captured
/// table stay exactly as they were (FR-NEW-002).
#[tokio::test]
async fn spec_0020_extract_text_sibling_over_the_limit_is_refused() {
    let w = World::shared().await;
    let extract_args = |refresh: bool| ExtractTextArgs {
        mount_id: ATLAS.into(),
        path: "/wide.csv".into(),
        max_chars: 200_000,
        preview_chars: 4_000,
        ocr: false,
        refresh,
    };

    // A small source whose first conversion leaves a sibling and a captured table.
    w.put_text("/wide.csv", "a\n1").await;
    let out = ok_json(w.as_(MIA).fs_extract_text(Parameters(extract_args(false))).await);
    assert_eq!(out["md_path"], json!("/wide.md"), "got: {out}");
    let old_sibling = w.content("/wide.md").await.expect("the first sibling");
    assert_eq!(old_sibling, one_column_table_md("a", "1").into_bytes());
    let old_sibling_len = old_sibling.len() as i64;
    assert_eq!(old_sibling_len, 19);
    assert_eq!(w.used().await, 1_000_022);

    // Rewrite the source larger: a fresh write of it alone still fits under
    // the limit, but the new sibling it produces grows far more than that
    // over the sibling already there. The rewrite bumps the source's
    // revision, which already orphans the table captured against the old
    // one (FR-NEW-023, the current set is only valid while the node still
    // carries the revision it was built from): the refused re-extraction
    // below must leave that exactly as the rewrite alone left it.
    let long = "x".repeat(48_000);
    w.put_text("/wide.csv", &format!("a\n{long}")).await;
    let used_before = w.used().await;
    assert_eq!(used_before, 1_048_021);

    let list_tables_args =
        || ListTablesArgs { mount_id: ATLAS.into(), path: "/wide.csv".into(), marker: None };
    let tables_after_rewrite =
        err_text(w.as_(MIA).fs_list_tables(Parameters(list_tables_args())).await);
    assert!(
        tables_after_rewrite.ends_with("No artifacts: document not extracted"),
        "got: {tables_after_rewrite}"
    );

    let requested_md = one_column_table_md("a", &long);
    assert_eq!(requested_md.len(), 48_018);
    let delta = requested_md.len() as i64 - old_sibling_len;
    assert_eq!(delta, 47_999);

    let err = err_text(w.as_(MIA).fs_extract_text(Parameters(extract_args(true))).await);
    assert_eq!(
        err,
        format!("An error occurred invoking 'fs.extract_text': {}", refusal(used_before, delta)),
        "got: {err}"
    );

    assert_eq!(w.content("/wide.md").await, Some(old_sibling), "sibling bytes unchanged");
    let tables_after_refusal =
        err_text(w.as_(MIA).fs_list_tables(Parameters(list_tables_args())).await);
    assert_eq!(
        tables_after_refusal, tables_after_rewrite,
        "table listing unchanged by the refusal"
    );
    assert_eq!(w.used().await, used_before, "usage unchanged except for the source rewrite");
}

// ── US-0018: archive and folder copy all or nothing ──────────────────────────

impl World {
    /// Setup write of raw bytes, straight into the volume (no session budget
    /// spent, mirrors [`Self::seed`]).
    async fn put(&self, path: &str, bytes: &[u8]) {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.write_bytes_atomic(path, bytes).await.unwrap();
    }

    async fn exists(&self, path: &str) -> bool {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        client.exists(path).await.unwrap()
    }

    async fn extract(&self, person: &str, path: &str) -> RawResult {
        let args = ExtractArchiveArgs {
            mount_id: ATLAS.into(),
            path: path.into(),
            destination: None,
            overwrite: false,
            password: None,
        };
        self.as_(person).fs_extract_archive(Parameters(args)).await
    }

    async fn copy(
        &self,
        person: &str,
        source: &str,
        destination: &str,
        recursive: bool,
    ) -> RawResult {
        let args = CopyArgs {
            mount_id: ATLAS.into(),
            source: source.into(),
            destination: destination.into(),
            overwrite: false,
            recursive,
        };
        self.as_(person).fs_copy(Parameters(args)).await
    }
}

/// A zip with every entry stored uncompressed, so its total declared size is
/// exactly the sum of `entries`' content lengths.
fn build_zip_entries(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut writer, content).unwrap();
        }
        writer.finish().unwrap();
    }
    buf
}

/// Atlas at its 1 MB limit, usage exactly 1 000 000 bytes, `/pack.zip`
/// (whose bytes are part of that 1 000 000) holding `entries`.
async fn archive_world(entries: &[(&str, Vec<u8>)]) -> (World, Vec<u8>) {
    let zip = build_zip_entries(entries);
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 1_000_000 - zip.len()).await;
    w.put("/pack.zip", &zip).await;
    assert_eq!(w.used().await, 1_000_000);
    (w, zip)
}

/// Atlas at its 1 MB limit, usage exactly 1 000 000 bytes, `/src` holding
/// `file_sizes.len()` files totalling `file_sizes`' sum (part of that
/// 1 000 000).
async fn folder_world(file_sizes: &[usize]) -> World {
    let total: usize = file_sizes.iter().sum();
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 1_000_000 - total).await;
    for (i, size) in file_sizes.iter().enumerate() {
        w.seed(&format!("/src/f{i}.bin"), *size).await;
    }
    assert_eq!(w.used().await, 1_000_000);
    w
}

/// SPEC-0020/E2E-027: an extraction whose entries fit under the limit is
/// accepted and counted once, by their net total (FR-NEW-007, FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_027_an_extraction_that_fits_is_accepted_and_counted() {
    let (w, _zip) =
        archive_world(&[("x", noise(10_000, 1)), ("y", noise(10_000, 2)), ("z", noise(20_000, 3))])
            .await;

    let out = ok_json(w.extract(MIA, "/pack.zip").await);
    assert_eq!(out["destination"], json!("/pack"), "got: {out}");
    assert_eq!(w.content("/pack/x").await.map(|b| b.len()), Some(10_000));
    assert_eq!(w.content("/pack/y").await.map(|b| b.len()), Some(10_000));
    assert_eq!(w.content("/pack/z").await.map(|b| b.len()), Some(20_000));
    assert_eq!(w.used().await, 1_040_000);
}

/// SPEC-0020/E2E-028: an extraction whose entries total over the limit is
/// refused as one operation, writing none of its files (FR-NEW-007,
/// FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_028_an_extraction_over_the_limit_writes_nothing() {
    let (w, _zip) =
        archive_world(&[("x", noise(20_000, 1)), ("y", noise(20_000, 2)), ("z", noise(10_000, 3))])
            .await;

    let err = err_text(w.extract(MIA, "/pack.zip").await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    assert!(!w.exists("/pack").await, "the archive's folder was not created");
    assert!(!w.exists("/pack/x").await, "no node at the refused path");
    assert!(!w.exists("/pack/y").await, "no node at the refused path");
    assert!(!w.exists("/pack/z").await, "no node at the refused path");
    assert_eq!(w.used().await, 1_000_000);

    // SPEC-0020/E2E-062: the refusal charged nothing to the session budget
    // (FR-NEW-007, FR-NEW-006); once the project limit is lifted, the full
    // session budget is still available, exactly as before the refusal.
    w.remove_limit().await;
    w.write(MIA, "/full.bin", SESSION_BUDGET as usize).await;
    let err = err_text(w.try_write(MIA, "/one.bin", 1).await);
    assert!(err.ends_with(SESSION_REFUSAL), "got: {err}");
}

/// SPEC-0020/E2E-029: an extraction whose entries total exactly the
/// remaining headroom is accepted (FR-NEW-007, FR-NEW-005).
#[tokio::test]
async fn spec_0020_e2e_029_an_extraction_to_exactly_the_limit_is_accepted() {
    let (w, _zip) =
        archive_world(&[("x", noise(20_000, 1)), ("y", noise(20_000, 2)), ("z", noise(8_576, 3))])
            .await;

    ok_json(w.extract(MIA, "/pack.zip").await);
    assert_eq!(w.used().await, MB);
}

/// SPEC-0020/E2E-030: a recursive folder copy whose total over the limit is
/// refused as one operation, leaving the destination absent (FR-NEW-007,
/// FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_030_a_folder_copy_over_the_limit_writes_nothing() {
    let w = folder_world(&[20_000, 20_000, 20_000]).await;

    let err = err_text(w.copy(MIA, "/src", "/dst", true).await);
    assert!(err.ends_with(&refusal(1_000_000, 60_000)), "got: {err}");
    assert!(!w.exists("/dst").await, "the destination was not created");
    assert_eq!(w.used().await, 1_000_000);
}

/// A tar archive (uncompressed) with `entries` written in order; a path may
/// appear more than once, exactly as `admit_paths`' caller (`extract_archive`)
/// writes it: in archive order, so the LAST occurrence is what ends up on
/// disk (FR-NEW-007).
fn build_tar_entries(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).unwrap();
            header.set_size(content.len() as u64);
            header.set_cksum();
            builder.append(&header, content.as_slice()).unwrap();
        }
        builder.finish().unwrap();
    }
    buf
}

/// Atlas at its 1 MB limit, usage exactly 1 000 000 bytes (so 48 576 bytes
/// of headroom), `/pack.tar` (whose bytes are part of that 1 000 000)
/// holding `entries`.
async fn tar_world(entries: &[(&str, Vec<u8>)]) -> (World, Vec<u8>) {
    let tar = build_tar_entries(entries);
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 1_000_000 - tar.len()).await;
    w.put("/pack.tar", &tar).await;
    assert_eq!(w.used().await, 1_000_000);
    (w, tar)
}

/// US-0018 (SPEC-0020/FR-NEW-007): a tar archive listing the same destination path twice
/// (tar permits duplicate entries; the last one written wins, FR-NEW-007) is
/// admitted by its NET change, the last occurrence's size minus whatever sat
/// there before, not the sum of every occurrence. Atlas has exactly 48 576
/// bytes of headroom; the archive's two occurrences of `/pack/x` sum to
/// 50 000 bytes (over headroom) but net to 20 000 (under it, last write
/// wins), so the extraction is accepted and usage grows by 20 000 only.
#[tokio::test]
async fn spec_0020_us_0018_duplicate_tar_path_netted_not_summed() {
    let (w, _tar) = tar_world(&[("x", noise(30_000, 1)), ("x", noise(20_000, 2))]).await;

    ok_json(w.extract(MIA, "/pack.tar").await);
    assert_eq!(w.content("/pack/x").await.map(|b| b.len()), Some(20_000));
    assert_eq!(w.used().await, 1_020_000);
}

/// US-0018 (SPEC-0020/FR-NEW-007): the same duplicate path tar, but the LAST occurrence
/// alone already exceeds the 48 576 bytes of headroom. The refusal reports
/// the netted requested bytes (50 000), never the summed total of every
/// occurrence (60 000).
#[tokio::test]
async fn spec_0020_us_0018_duplicate_tar_path_refused_with_netted_total() {
    let (w, _tar) = tar_world(&[("x", noise(10_000, 1)), ("x", noise(50_000, 2))]).await;

    let err = err_text(w.extract(MIA, "/pack.tar").await);
    assert!(err.ends_with(&refusal(1_000_000, 50_000)), "got: {err}");
    assert!(!w.exists("/pack").await, "the archive's folder was not created");
    assert!(!w.exists("/pack/x").await, "no node at the refused path");
    assert_eq!(w.used().await, 1_000_000);
}

// ── US-0019: version control operations under the limit ─────────────────────

use crate::errors::ToolError;
use crate::git::GitRepoStore;
use crate::mcp::server::{
    GitBranchCreateArgs, GitBranchSwitchArgs, GitCheckoutFileArgs, GitCherryPickArgs, GitMergeArgs,
    GitStashIdArgs, GitStashSaveArgs,
};
use crate::tools::git as vc;

/// The refusal message of FR-NEW-001 as a version control tool returns it.
fn vc_refusal(used: i64, requested: i64) -> String {
    format!(
        "project storage quota of 1 MB exceeded: {used} bytes used, {requested} bytes requested"
    )
}

fn assert_refused(r: crate::errors::Result<Value>, used: i64, requested: i64) {
    let e: ToolError = r.expect_err("the operation must be refused");
    assert_eq!(e.code, "ERR_PROJECT_QUOTA_EXCEEDED", "got: {e:?}");
    assert_eq!(e.message, vc_refusal(used, requested));
}

/// Atlas with version control on. Setup steps (commits, branch moves) run as
/// Olivia, so Mia's session budget is untouched when she runs the operation
/// under test.
struct Vc {
    w: World,
    store: Arc<GitRepoStore>,
}

impl Vc {
    async fn new() -> Vc {
        let w = World::new(true).await;
        let store = Arc::new(GitRepoStore::new(
            w.f.state.config.clone(),
            w.f.state.stores.relational().clone(),
        ));
        Vc { w, store }
    }

    fn g(&self) -> Option<Arc<GitRepoStore>> {
        Some(self.store.clone())
    }

    async fn init(&self) {
        let init = MountOnlyArgs { mount_id: ATLAS.into() };
        vc::tool_init(self.w.f.ctx(OLIVIA), init, self.g()).await.unwrap();
    }

    async fn commit(&self, message: &str) -> String {
        let args = GitCommitArgs {
            mount_id: ATLAS.into(),
            message: message.into(),
            author_name: None,
            author_email: None,
        };
        let out = vc::tool_commit(self.w.f.ctx(OLIVIA), args, self.g()).await.unwrap();
        out["commit_sha"].as_str().expect("a commit sha").to_string()
    }

    /// Olivia creates `name` at HEAD and checks it out.
    async fn branch_out(&self, name: &str) {
        let args = GitBranchCreateArgs {
            mount_id: ATLAS.into(),
            name: name.into(),
            start_point: String::new(),
            checkout: true,
        };
        vc::tool_branch_create(self.w.f.ctx(OLIVIA), args, self.g()).await.unwrap();
    }

    async fn switch(&self, person: &str, name: &str) -> crate::errors::Result<Value> {
        let args = GitBranchSwitchArgs { mount_id: ATLAS.into(), name: name.into() };
        vc::tool_branch_switch(self.w.f.ctx(person), args, self.g()).await
    }

    async fn head(&self) -> String {
        let entry = self.store.get_or_open_repo(ATLAS).await.unwrap();
        entry.db.get_ref("HEAD").await.unwrap().expect("a HEAD").target
    }

    async fn delete(&self, path: &str) {
        let client = self.w.f.state.stores.client(ATLAS).await.unwrap();
        client.delete_file(path).await.unwrap();
    }

    async fn size(&self, path: &str) -> Option<usize> {
        self.w.content(path).await.map(|b| b.len())
    }
}

/// A bare repository at `dir` whose `main` is one commit per tree in `trees`,
/// in order, each tree replacing the previous one; returns its `file://` url.
fn vc_remote(dir: &std::path::Path, trees: &[&[(&str, usize)]]) -> String {
    let repo = git2::Repository::init_bare(dir).unwrap();
    for files in trees {
        vc_remote_commit(&repo, files);
    }
    repo.set_head("refs/heads/main").unwrap();
    format!("file://{}", dir.display())
}

/// One more commit on `repo`'s `main` whose tree is exactly `files`.
fn vc_remote_commit(repo: &git2::Repository, files: &[(&str, usize)]) {
    let parent = repo.find_reference("refs/heads/main").ok().map(|r| r.peel_to_commit().unwrap());
    let mut builder = repo.treebuilder(None).unwrap();
    for (name, len) in files {
        let blob = repo.blob(&noise(*len, *len as u64 ^ name.len() as u64)).unwrap();
        builder.insert(*name, blob, 0o100_644).unwrap();
    }
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let sig =
        git2::Signature::new("Origin", "o@t.com", &git2::Time::new(1_700_000_000, 0)).unwrap();
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(Some("refs/heads/main"), &sig, &sig, "remote\n", &tree, &parents).unwrap();
}

/// SPEC-0020/E2E-031: a clone whose files exceed the room left is refused and
/// imports no file (FR-NEW-003, FR-NEW-007).
#[tokio::test]
async fn spec_0020_e2e_031_a_clone_over_the_limit_imports_nothing() {
    let v = Vc::new().await;
    v.w.limit(Some(MB)).await;
    v.w.seed("/seed.bin", 1_000_000).await;
    let url =
        vc_remote(&v.w.f.dir.path().join("remote-031"), &[&[("a.bin", 30_000), ("b.bin", 30_000)]]);

    let r = vc::test_support::clone_from(&v.w.f.ctx(MIA), v.store.clone(), ATLAS, &url).await;
    assert_refused(r, 1_000_000, 60_000);
    assert!(v.w.content("/a.bin").await.is_none());
    assert!(v.w.content("/b.bin").await.is_none());
    assert_eq!(v.w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-036: a clone counts its files, never its history
/// (FR-NEW-002, FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_036_a_clone_counts_files_not_history() {
    let v = Vc::new().await;
    v.w.limit(Some(MB)).await;
    v.w.seed("/seed.bin", 1_000_000).await;
    let old: &[(&str, usize)] = &[("old.bin", 460_000)];
    let now: &[(&str, usize)] = &[("a.bin", 20_000), ("b.bin", 20_000)];
    let url = vc_remote(&v.w.f.dir.path().join("remote-036"), &[old, now]);

    vc::test_support::clone_from(&v.w.f.ctx(MIA), v.store.clone(), ATLAS, &url).await.unwrap();
    assert_eq!(v.size("/a.bin").await, Some(20_000));
    assert_eq!(v.w.used().await, 1_040_000);
}

/// SPEC-0020/E2E-032: a pull bringing more than the room left is refused and
/// the files stay at their previous commit (FR-NEW-007, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_032_a_pull_over_the_limit_keeps_the_previous_commit() {
    let v = Vc::new().await;
    let dir = v.w.f.dir.path().join("remote-032");
    let url = vc_remote(&dir, &[&[("c.txt", 10)]]);
    vc::test_support::clone_from(&v.w.f.ctx(OLIVIA), v.store.clone(), ATLAS, &url).await.unwrap();
    // Local work committed on top of the clone brings usage to 1 000 000.
    v.w.seed("/seed.bin", 999_990).await;
    v.commit("local").await;
    v.w.limit(Some(MB)).await;
    let remote = git2::Repository::open_bare(&dir).unwrap();
    vc_remote_commit(&remote, &[("c.txt", 10), ("new.bin", 50_000)]);
    let before = v.w.content("/c.txt").await;

    let r = vc::test_support::pull_from(&v.w.f.ctx(MIA), &v.store, ATLAS, &url, "main").await;
    assert_refused(r, 1_000_000, 50_000);
    assert!(v.w.content("/new.bin").await.is_none());
    assert_eq!(v.w.content("/c.txt").await, before);
    assert_eq!(v.w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-033: a merge adding more than the room left is refused and
/// the target branch's files are unchanged (FR-NEW-003, FR-NEW-007).
#[tokio::test]
async fn spec_0020_e2e_033_a_merge_over_the_limit_leaves_main_unchanged() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 1_000_000).await;
    v.init().await;
    v.commit("base").await;
    v.branch_out("feature").await;
    v.w.seed("/f.bin", 49_000).await;
    v.commit("feature").await;
    v.switch(OLIVIA, "main").await.unwrap();
    v.w.limit(Some(MB)).await;
    let seed = v.w.content("/seed.bin").await;

    let args = GitMergeArgs {
        mount_id: ATLAS.into(),
        source_ref: "feature".into(),
        squash: false,
        message: None,
    };
    assert_refused(vc::tool_merge(v.w.f.ctx(MIA), args, v.g()).await, 1_000_000, 49_000);
    assert!(v.w.content("/f.bin").await.is_none());
    assert_eq!(v.w.content("/seed.bin").await, seed);
    assert_eq!(v.w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-034: a branch switch growing a file past the room left is
/// refused; the branch and its files stay (FR-NEW-003, FR-NEW-007).
#[tokio::test]
async fn spec_0020_e2e_034_a_switch_over_the_limit_keeps_the_branch() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 900_000).await;
    v.w.seed("/r.md", 100_000).await;
    v.init().await;
    v.commit("base").await;
    v.branch_out("big").await;
    v.w.seed("/r.md", 150_000).await;
    v.commit("big").await;
    v.switch(OLIVIA, "main").await.unwrap();
    v.w.limit(Some(MB)).await;
    let r_md = v.w.content("/r.md").await;
    assert_eq!(v.w.used().await, 1_000_000);

    assert_refused(v.switch(MIA, "big").await, 1_000_000, 50_000);
    assert_eq!(v.head().await, "refs/heads/main");
    assert_eq!(v.w.content("/r.md").await, r_md);
    assert_eq!(v.w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-035: a switch that shrinks the project is accepted even over
/// the limit (FR-NEW-004).
#[tokio::test]
async fn spec_0020_e2e_035_a_shrinking_switch_over_the_limit_is_accepted() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 1_300_000).await;
    v.w.seed("/x.bin", 200_000).await;
    v.init().await;
    v.commit("base").await;
    v.branch_out("slim").await;
    v.delete("/x.bin").await;
    v.commit("slim").await;
    v.switch(OLIVIA, "main").await.unwrap();
    v.w.limit(Some(MB)).await;
    assert_eq!(v.w.used().await, 1_500_000);

    v.switch(MIA, "slim").await.unwrap();
    assert_eq!(v.w.used().await, 1_300_000);
    assert_eq!(v.w.independent_sum().await, 1_300_000);
}

/// SPEC-0020/E2E-065: a cherry pick over the limit is refused and writes
/// nothing (FR-NEW-003, FR-NEW-007).
#[tokio::test]
async fn spec_0020_e2e_065_a_cherry_pick_over_the_limit_writes_nothing() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 1_500_000).await;
    v.init().await;
    v.commit("base").await;
    v.branch_out("feature").await;
    v.w.seed("/k.txt", 10).await;
    let pick = v.commit("k").await;
    v.switch(OLIVIA, "main").await.unwrap();
    v.w.limit(Some(MB)).await;

    let args = GitCherryPickArgs { mount_id: ATLAS.into(), commit_sha: pick, mainline: 0 };
    let r = vc::tool_cherry_pick(v.w.f.ctx(MIA), args, 0, v.g()).await;
    assert_refused(r, 1_500_000, 10);
    assert!(v.w.content("/k.txt").await.is_none());
    assert_eq!(v.w.used().await, 1_500_000);
}

/// SPEC-0020/E2E-066: a stash pop over the limit is refused, writes nothing
/// and keeps the stash (FR-NEW-003, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_066_a_stash_pop_over_the_limit_keeps_the_stash() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 1_000_000).await;
    v.init().await;
    v.commit("base").await;
    v.w.seed("/st.txt", 50_000).await;
    let args = GitStashSaveArgs { mount_id: ATLAS.into(), message: None };
    let saved = vc::tool_stash_save(v.w.f.ctx(OLIVIA), args, v.g()).await.unwrap();
    let stash_id = saved["stash_id"].as_str().expect("a stash id").to_string();
    v.w.limit(Some(MB)).await;
    assert_eq!(v.w.used().await, 1_000_000);

    let args = GitStashIdArgs { mount_id: ATLAS.into(), stash_id };
    let r = vc::tool_stash_apply_or_pop(v.w.f.ctx(MIA), args, v.g(), "git.stash_pop", true).await;
    assert_refused(r, 1_000_000, 50_000);
    assert!(v.w.content("/st.txt").await.is_none());
    let list = MountOnlyArgs { mount_id: ATLAS.into() };
    let stashes = vc::tool_stash_list(v.w.f.ctx(MIA), list, v.g()).await.unwrap();
    assert_eq!(stashes["count"], 1, "the stash still exists: {stashes}");
}

/// SPEC-0020/E2E-067: a removed path leaves the usage at once, not to the
/// trash, so a switch whose net change is negative is accepted
/// (FR-NEW-003, FR-NEW-004).
#[tokio::test]
async fn spec_0020_e2e_067_a_removed_path_leaves_the_usage_at_once() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 1_400_000).await;
    v.w.seed("/r.md", 100_000).await;
    v.init().await;
    v.commit("base").await;
    v.branch_out("swap").await;
    v.delete("/r.md").await;
    v.w.seed("/n2.txt", 50_000).await;
    v.commit("swap").await;
    v.switch(OLIVIA, "main").await.unwrap();
    v.w.limit(Some(MB)).await;
    assert_eq!(v.w.used().await, 1_500_000);

    v.switch(MIA, "swap").await.unwrap();
    assert!(v.w.content("/r.md").await.is_none());
    assert_eq!(v.size("/n2.txt").await, Some(50_000));
    assert_eq!(v.w.used().await, 1_450_000);
    // A trashed `/r.md` would still be counted by the sum over the tree.
    assert_eq!(v.w.independent_sum().await, 1_450_000);
}

/// SPEC-0020/E2E-068: a merge abort is accepted whatever the limit
/// (FR-NEW-012).
#[tokio::test]
async fn spec_0020_e2e_068_a_merge_abort_is_accepted_over_the_limit() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 1_299_990).await;
    v.w.seed("/big.bin", 100_000).await;
    v.w.seed_text("/c.txt", "base text\n").await;
    v.init().await;
    v.commit("base").await;
    v.branch_out("feature").await;
    v.delete("/big.bin").await;
    v.w.seed_text("/c.txt", "featurexx\n").await;
    v.commit("feature").await;
    v.switch(OLIVIA, "main").await.unwrap();
    v.w.seed_text("/c.txt", "main text\n").await;
    v.commit("main").await;
    assert_eq!(v.w.used().await, 1_400_000);

    let args = GitMergeArgs {
        mount_id: ATLAS.into(),
        source_ref: "feature".into(),
        squash: false,
        message: None,
    };
    let out = vc::tool_merge(v.w.f.ctx(MIA), args, v.g()).await.unwrap();
    assert_eq!(out["status"], "conflict", "got: {out}");
    v.w.limit(Some(MB)).await;

    let abort = MountOnlyArgs { mount_id: ATLAS.into() };
    let out = vc::tool_merge_abort(v.w.f.ctx(MIA), abort, v.g()).await.unwrap();
    assert_eq!(out["status"], "aborted", "got: {out}");
    assert_eq!(v.size("/big.bin").await, Some(100_000));
    assert_eq!(v.w.used().await, 1_400_000);
}

/// SPEC-0020/E2E-069: restoring a file from a commit is checked for its
/// growth (FR-NEW-003, FR-NEW-013).
#[tokio::test]
async fn spec_0020_e2e_069_a_file_restore_over_the_limit_is_refused() {
    let v = Vc::new().await;
    v.w.seed("/seed.bin", 900_000).await;
    v.w.seed("/r.md", 150_000).await;
    v.init().await;
    let last = v.commit("base").await;
    v.w.seed("/r.md", 100_000).await;
    v.w.limit(Some(MB)).await;
    assert_eq!(v.w.used().await, 1_000_000);

    let args =
        GitCheckoutFileArgs { mount_id: ATLAS.into(), commit_sha: last, path: "/r.md".into() };
    assert_refused(vc::tool_checkout_file(v.w.f.ctx(MIA), args, v.g()).await, 1_000_000, 50_000);
    assert_eq!(v.size("/r.md").await, Some(100_000));
    assert_eq!(v.w.used().await, 1_000_000);
}

// ── US-0020: lowered limit, exempt operations, concurrency ──────────────────

use crate::mcp::server::{MoveArgs, TrashRestoreArgs};

impl World {
    async fn delete_to_trash(&self, person: &str, path: &str) -> RawResult {
        let args =
            DeleteArgs { mount_id: ATLAS.into(), path: path.into(), recursive: false, trash: true };
        self.as_(person).fs_delete(Parameters(args)).await
    }

    async fn move_(&self, person: &str, source: &str, destination: &str) -> RawResult {
        let args = MoveArgs {
            mount_id: ATLAS.into(),
            source: source.into(),
            destination: destination.into(),
            overwrite: false,
        };
        self.as_(person).fs_move(Parameters(args)).await
    }

    /// The current path of the one trashed file whose original name is
    /// `name`: trash paths are generated, so they are looked up in the tree.
    async fn trashed(&self, name: &str) -> String {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        let hits: Vec<String> = client
            .meta
            .subtree("/")
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.is_file() && n.path != format!("/{name}") && n.path.contains(name))
            .map(|n| n.path)
            .collect();
        assert_eq!(hits.len(), 1, "one trashed {name}: {hits:?}");
        hits[0].clone()
    }

    /// Ada sets Atlas's limit through the tool, as FR-MOD-001 describes.
    async fn admin_limit(&self, max_mb: Option<u32>) {
        let args = SetProjectQuotaArgs { project_id: ATLAS.into(), max_mb };
        ok_json(self.as_(ADA).admin_set_project_quota(Parameters(args)).await);
    }

    /// Atlas with no limit using exactly 1 500 000 bytes: `/r.md` (100 000
    /// bytes of text), `/a.txt` (40 000), `/b.txt` (10 000), `/s.bin` (10).
    async fn over() -> World {
        let w = World::new(false).await;
        w.seed("/seed.bin", 1_349_990).await;
        w.seed_text("/r.md", &text(100_000, 3)).await;
        w.seed("/a.txt", 40_000).await;
        w.seed("/b.txt", 10_000).await;
        w.seed("/s.bin", 10).await;
        assert_eq!(w.used().await, 1_500_000);
        w
    }

    /// [`Self::over`] after Ada lowered the limit to 1 MB (E2E-044's state).
    async fn lowered() -> World {
        let w = World::over().await;
        w.admin_limit(Some(1)).await;
        w
    }
}

/// Whether a tool call succeeded, for races where either side may win.
fn accepted(r: &RawResult) -> bool {
    r.as_ref().expect("a tool result, not a protocol error").is_error != Some(true)
}

/// Run `a` and `b` as two tasks released by one barrier, so both reach the
/// admission together; no sleep decides the interleaving.
async fn race<A, B>(a: A, b: B) -> (RawResult, RawResult)
where
    A: std::future::Future<Output = RawResult> + Send + 'static,
    B: std::future::Future<Output = RawResult> + Send + 'static,
{
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let (ba, bb) = (barrier.clone(), barrier);
    let ta = tokio::spawn(async move {
        ba.wait().await;
        a.await
    });
    let tb = tokio::spawn(async move {
        bb.wait().await;
        b.await
    });
    (ta.await.unwrap(), tb.await.unwrap())
}

/// A new file write of `len` bytes by `person`, as an owned future.
fn spawnable_write(
    w: &World,
    person: &str,
    path: &str,
    len: usize,
) -> impl std::future::Future<Output = RawResult> + Send + 'static {
    let server = w.as_(person);
    let args = WriteBytesArgs {
        mount_id: ATLAS.into(),
        path: path.into(),
        base64: base64::engine::general_purpose::STANDARD.encode(noise(len, 13)),
        overwrite: false,
        create_parents: true,
        trigger_documentation_service: false,
    };
    async move { server.fs_write_bytes(Parameters(args)).await }
}

/// SPEC-0020/E2E-022: a delete to the trash is accepted and keeps the bytes
/// counted (FR-NEW-002, FR-NEW-012).
#[tokio::test]
async fn spec_0020_e2e_022_a_delete_keeps_the_bytes_in_the_trash() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 900_000).await;
    w.seed("/old.txt", 100_000).await;
    ok_json(w.delete_to_trash(MIA, "/old.txt").await);
    assert!(!w.exists("/old.txt").await);
    let trashed = w.trashed("old.txt").await;
    assert_eq!(w.content(&trashed).await.map(|b| b.len()), Some(100_000));
    assert_eq!(w.used().await, 1_000_000);
}

/// SPEC-0020/E2E-023: a permanent purge lowers the usage and frees room
/// (FR-NEW-012, FR-NEW-002).
#[tokio::test]
async fn spec_0020_e2e_023_a_purge_frees_room() {
    let w = World::new(false).await;
    w.limit(Some(MB)).await;
    w.seed("/seed.bin", 900_000).await;
    w.seed("/old.txt", 100_000).await;
    ok_json(w.delete_to_trash(OLIVIA, "/old.txt").await);
    let trashed = w.trashed("old.txt").await;

    let purge =
        DeleteArgs { mount_id: ATLAS.into(), path: trashed, recursive: false, trash: false };
    ok_json(w.as_(OLIVIA).fs_delete(Parameters(purge)).await);
    assert_eq!(w.used().await, 900_000);

    w.write(MIA, "/new.bin", 148_576).await;
    assert_eq!(w.used().await, MB);
}

/// SPEC-0020/E2E-024: a move over the limit is accepted and changes nothing
/// (FR-NEW-012).
#[tokio::test]
async fn spec_0020_e2e_024_a_move_over_the_limit_is_accepted() {
    let w = World::new(false).await;
    w.seed("/seed.bin", 1_460_000).await;
    w.seed("/a.txt", 40_000).await;
    w.seed("/arch/keep", 0).await;
    w.limit(Some(MB)).await;
    ok_json(w.move_(MIA, "/a.txt", "/arch/a.txt").await);
    assert!(w.exists("/arch/a.txt").await && !w.exists("/a.txt").await);
    assert_eq!(w.used().await, 1_500_000);
}

/// SPEC-0020/E2E-025: a trash restore over the limit is accepted and
/// changes nothing (FR-NEW-012).
#[tokio::test]
async fn spec_0020_e2e_025_a_restore_over_the_limit_is_accepted() {
    let w = World::new(false).await;
    w.seed("/seed.bin", 1_470_000).await;
    w.seed("/t.txt", 30_000).await;
    ok_json(w.delete_to_trash(OLIVIA, "/t.txt").await);
    let trashed = w.trashed("t.txt").await;
    w.limit(Some(MB)).await;
    assert_eq!(w.used().await, 1_500_000);

    let args = TrashRestoreArgs { mount_id: ATLAS.into(), trash_path: trashed };
    ok_json(w.as_(MIA).fs_trash_restore(Parameters(args)).await);
    assert_eq!(w.content("/t.txt").await.map(|b| b.len()), Some(30_000));
    assert_eq!(w.used().await, 1_500_000);
}

/// SPEC-0020/E2E-026: a push over the limit succeeds with no quota message
/// (FR-NEW-012). Driven through the real git HTTP smart server
/// (`git::http::router`, `POST /git/Atlas/git-receive-pack`) with Mia's
/// bearer token, carrying a pack built by a scratch libgit2 repository.
#[tokio::test]
async fn spec_0020_e2e_026_a_push_over_the_limit_succeeds() {
    use crate::git::http::pktline;
    use tower::ServiceExt as _;

    let w = World::new(true).await;
    w.seed("/seed.bin", 1_500_000).await;
    w.limit(Some(MB)).await;
    let keys = tempfile::tempdir().unwrap();
    let (key, public) = crate::keys::write_keypair(keys.path()).unwrap();
    let mut config = (*w.f.state.config).clone();
    config.auth.jwt.public_key_path = public.display().to_string();
    let config = Arc::new(config);
    let s = &w.f.state;
    let state = AppState {
        config: config.clone(),
        admin: s.admin.clone(),
        stores: s.stores.clone(),
        safety: s.safety.clone(),
        identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
        editors: s.editors.clone(),
        doc_service: None,
        search: None,
    };
    let git = Arc::new(GitRepoStore::new(config, s.stores.relational().clone()));
    git.init_repo(ATLAS).await.unwrap();
    let app = crate::git::http::router(Arc::new(state), git.clone());
    let token = crate::keys::mint_token_from_file(
        &key,
        MIA,
        crate::keys::DEFAULT_ISSUER,
        crate::keys::DEFAULT_CLAIM,
        3600,
    )
    .unwrap();

    // Two new commits, 200 000 bytes of new content, built client side.
    let src_dir = tempfile::tempdir().unwrap();
    let src = git2::Repository::init_bare(src_dir.path()).unwrap();
    vc_remote_commit(&src, &[("one.bin", 100_000)]);
    vc_remote_commit(&src, &[("one.bin", 100_000), ("two.bin", 100_000)]);
    let tip = src.refname_to_id("refs/heads/main").unwrap();
    let mut pack = git2::Buf::new();
    {
        let mut pb = src.packbuilder().unwrap();
        let mut walk = src.revwalk().unwrap();
        walk.push(tip).unwrap();
        pb.insert_walk(&mut walk).unwrap();
        pb.write_buf(&mut pack).unwrap();
    }
    let mut body = pktline::encode_raw(
        format!("{} {tip} refs/heads/main\0report-status\n", "0".repeat(40)).as_bytes(),
    );
    body.extend_from_slice(&pktline::flush());
    body.extend_from_slice(&pack);

    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/git/{ATLAS}/git-receive-pack"))
        .header("X-Forwarded-Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/x-git-receive-pack-request")
        .body(axum::body::Body::from(body))
        .unwrap();
    let r = app.oneshot(req).await.unwrap();
    assert_eq!(r.status(), 200);
    let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
    let report = String::from_utf8_lossy(&bytes);
    assert!(report.contains("unpack ok"), "got: {report}");
    assert!(report.contains("ok refs/heads/main"), "got: {report}");
    assert!(!report.contains("quota"), "got: {report}");
    let entry = git.get_or_open_repo(ATLAS).await.unwrap();
    assert_eq!(entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target, tip.to_string());
    assert_eq!(w.used().await, 1_500_000);
}

/// SPEC-0020/E2E-044: a limit below the usage is accepted and removes
/// nothing (FR-MOD-001).
#[tokio::test]
async fn spec_0020_e2e_044_a_limit_below_the_usage_removes_nothing() {
    let w = World::over().await;
    w.admin_limit(Some(1)).await;
    for path in ["/seed.bin", "/r.md", "/a.txt", "/b.txt", "/s.bin"] {
        assert!(w.exists(path).await, "{path} still present");
    }
    assert_eq!(w.independent_sum().await, 1_500_000);
    assert_eq!(
        ok_json(w.quota(ADA, ATLAS).await),
        json!({"project_id": "Atlas", "max_mb": 1, "used_bytes": 1_500_000})
    );
}

/// SPEC-0020/E2E-045: after the lowering any growth is refused
/// (FR-MOD-001, FR-NEW-001).
#[tokio::test]
async fn spec_0020_e2e_045_a_one_byte_write_is_refused_after_the_lowering() {
    let w = World::lowered().await;
    let err = err_text(w.try_write(MIA, "/one.txt", 1).await);
    assert!(err.ends_with(&refusal(1_500_000, 1)), "got: {err}");
    assert!(!w.exists("/one.txt").await);
}

/// SPEC-0020/E2E-046: a shrinking overwrite is accepted over the limit
/// (FR-MOD-001, FR-NEW-004).
#[tokio::test]
async fn spec_0020_e2e_046_a_shrinking_overwrite_is_accepted_over_the_limit() {
    let w = World::lowered().await;
    w.read_as_mia("/r.md").await;
    ok_json(w.try_write_text("/r.md", &text(40_000, 5), true).await);
    assert_eq!(w.used().await, 1_440_000);
}

/// SPEC-0020/E2E-047: a delete to the trash and a move are accepted over the
/// limit and change nothing (FR-MOD-001, FR-NEW-012).
#[tokio::test]
async fn spec_0020_e2e_047_delete_and_move_are_accepted_over_the_limit() {
    let w = World::lowered().await;
    ok_json(w.delete_to_trash(MIA, "/a.txt").await);
    ok_json(w.move_(MIA, "/b.txt", "/c.txt").await);
    assert!(!w.exists("/a.txt").await && !w.exists("/b.txt").await && w.exists("/c.txt").await);
    assert_eq!(w.used().await, 1_500_000);
}

/// SPEC-0020/E2E-048: a limit equal to the usage is accepted and leaves no
/// room (FR-MOD-001, FR-NEW-005).
#[tokio::test]
async fn spec_0020_e2e_048_a_limit_equal_to_the_usage_leaves_no_room() {
    let w = World::new(false).await;
    w.seed("/seed.bin", MB as usize).await;
    w.admin_limit(Some(1)).await;
    let err = err_text(w.try_write(MIA, "/one.txt", 1).await);
    assert!(err.ends_with(&refusal(MB, 1)), "got: {err}");
}

/// SPEC-0020/E2E-049 and E2E-063: of two writes that fit alone but not
/// together exactly one is accepted; the loser's session budget is intact
/// once the limit is gone (FR-NEW-008, FR-NEW-006).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spec_0020_e2e_049_063_one_of_two_racing_writes_is_refused() {
    let w = World::shared().await;
    let (mia, olivia) = race(
        spawnable_write(&w, MIA, "/mia.bin", 40_000),
        spawnable_write(&w, OLIVIA, "/olivia.bin", 40_000),
    )
    .await;
    assert_ne!(accepted(&mia), accepted(&olivia), "exactly one write is accepted");
    let (loser, loser_result) = if accepted(&mia) { (OLIVIA, olivia) } else { (MIA, mia) };
    let err = err_text(loser_result);
    assert!(err.ends_with(&refusal(1_040_000, 40_000)), "got: {err}");
    assert_eq!(w.used().await, 1_040_000);
    assert_eq!(w.independent_sum().await, 1_040_000);

    // E2E-063: the refusal charged nothing, so the whole budget is left.
    w.remove_limit().await;
    let r = spawnable_write(&w, loser, "/retry.bin", SESSION_BUDGET as usize).await;
    ok_json(r);
}

/// SPEC-0020/E2E-050: two racing writes that fit together are both accepted
/// (FR-NEW-008).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spec_0020_e2e_050_two_racing_writes_that_fit_are_accepted() {
    let w = World::shared().await;
    let (mia, olivia) = race(
        spawnable_write(&w, MIA, "/mia.bin", 24_000),
        spawnable_write(&w, OLIVIA, "/olivia.bin", 24_000),
    )
    .await;
    ok_json(mia);
    ok_json(olivia);
    assert_eq!(w.used().await, 1_048_000);
}

/// SPEC-0020/E2E-051: an extraction racing a write: exactly one lands, and a
/// refused extraction writes no entry (FR-NEW-008, FR-NEW-007).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spec_0020_e2e_051_an_extraction_racing_a_write() {
    let (w, _zip) = archive_world(&[("x", noise(10_000, 1)), ("y", noise(20_000, 2))]).await;
    let mia = w.as_(MIA);
    let extract = async move {
        let args = ExtractArchiveArgs {
            mount_id: ATLAS.into(),
            path: "/pack.zip".into(),
            destination: None,
            overwrite: false,
            password: None,
        };
        mia.fs_extract_archive(Parameters(args)).await
    };
    let (ex, wr) = race(extract, spawnable_write(&w, OLIVIA, "/w.bin", 30_000)).await;
    assert_ne!(accepted(&ex), accepted(&wr), "exactly one operation is accepted");
    assert_eq!(w.used().await, 1_030_000);
    if accepted(&ex) {
        assert!(err_text(wr).ends_with(&refusal(1_030_000, 30_000)));
        assert!(!w.exists("/w.bin").await);
    } else {
        assert!(err_text(ex).ends_with(&refusal(1_030_000, 30_000)));
        let client = w.f.state.stores.client(ATLAS).await.unwrap();
        let names: Vec<String> = client
            .meta
            .subtree("/")
            .await
            .unwrap()
            .into_iter()
            .map(|n| n.path)
            .filter(|p| p.ends_with("/x") || p.ends_with("/y"))
            .collect();
        assert!(names.is_empty(), "no entry of the refused extraction: {names:?}");
    }
}

/// SPEC-0020/E2E-052: 20 concurrent pairs of 8 000 byte writes with 8 576
/// bytes left: exactly one is accepted and the usage, sampled while they
/// run, never exceeds the limit (FR-NEW-008).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spec_0020_e2e_052_twenty_racing_pairs_admit_exactly_one() {
    let w = Arc::new(World::shared().await);
    w.seed("/fill.bin", 40_000).await;
    assert_eq!(w.used().await, 1_040_000);

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut writers = tokio::task::JoinSet::new();
    for person in [MIA, OLIVIA] {
        let (w, barrier) = (w.clone(), barrier.clone());
        writers.spawn(async move {
            barrier.wait().await;
            let mut ok = 0;
            for i in 0..20 {
                let path = format!("/{}-{i}.bin", &person[..3]);
                if accepted(&spawnable_write(&w, person, &path, 8_000).await) {
                    ok += 1;
                }
                tokio::task::yield_now().await;
            }
            ok
        });
    }
    let watcher = {
        let (w, barrier, done) = (w.clone(), barrier.clone(), done.clone());
        tokio::spawn(async move {
            barrier.wait().await;
            let mut peak = 0;
            while !done.load(std::sync::atomic::Ordering::Acquire) {
                peak = peak.max(w.used().await);
                tokio::task::yield_now().await;
            }
            peak
        })
    };
    let mut accepted_total = 0;
    while let Some(ok) = writers.join_next().await {
        accepted_total += ok.unwrap();
    }
    done.store(true, std::sync::atomic::Ordering::Release);
    let peak = watcher.await.unwrap();

    assert_eq!(accepted_total, 1, "exactly one write is accepted");
    assert_eq!(w.used().await, 1_048_000);
    assert_eq!(w.independent_sum().await, 1_048_000);
    assert!(peak <= MB, "usage peaked at {peak}");
}

/// SPEC-0020/E2E-061: a copy after the lowering is refused and writes
/// nothing (FR-MOD-001, FR-NEW-003).
#[tokio::test]
async fn spec_0020_e2e_061_a_copy_after_the_lowering_is_refused() {
    let w = World::lowered().await;
    let err = err_text(w.copy(MIA, "/s.bin", "/s3.bin", false).await);
    assert!(err.ends_with(&refusal(1_500_000, 10)), "got: {err}");
    assert!(!w.exists("/s3.bin").await);
}

/// SPEC-0020/E2E-064: two racing writes that each overshoot are both refused
/// with the same figures (FR-NEW-008).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spec_0020_e2e_064_two_racing_overshoots_are_both_refused() {
    let w = World::shared().await;
    w.seed("/fill.bin", 48_000).await;
    let (mia, olivia) = race(
        spawnable_write(&w, MIA, "/mia.bin", 1_000),
        spawnable_write(&w, OLIVIA, "/olivia.bin", 1_000),
    )
    .await;
    for r in [mia, olivia] {
        let err = err_text(r);
        assert!(err.ends_with(&refusal(1_048_000, 1_000)), "got: {err}");
    }
    assert_eq!(w.used().await, 1_048_000);
}
