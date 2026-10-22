//! SPEC-0049 acceptance tests: `admin.get_project_folder_usage`, driven through the
//! real `McpServer` tool method (and the data plane router for the web interface)
//! over real storage. Nothing is mocked.
//!
//! Shared setup: project `Atlas` has a limit of 2 MB and stores seven files
//! (1 225 500 bytes in all); Olivia owns it, Mia is a member, Ada is a platform
//! admin, Otto belongs to nothing, Bram owns `Borealis`.

use crate::mcp::server::{GetProjectFolderUsageArgs, McpServer};
use crate::state::AppState;
use crate::tools::admin::test_support::Fixture;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};
use std::sync::Arc;

const ATLAS: &str = "Atlas";
const BOREALIS: &str = "Borealis";
const MIA: &str = "mia@example.com";
const OLIVIA: &str = "olivia@example.com";
const ADA: &str = "ada@example.com";
const OTTO: &str = "otto@example.com";
const BRAM: &str = "bram@example.com";
const MB: i64 = 1_048_576;
const SESSION_BUDGET: i64 = 262_144;

const FILES: [(&str, usize); 7] = [
    ("/video/demo.mp4", 600_000),
    ("/video/clips/a.mp4", 200_000),
    ("/data/export.csv", 250_000),
    ("/data/old/x.csv", 50_000),
    ("/docs/spec.docx", 120_000),
    ("/docs/notes.txt", 4_000),
    ("/readme.md", 1_500),
];

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

/// Bytes that do not compress, so no test depends on content sharing by accident.
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

struct World {
    f: Fixture,
}

impl World {
    /// Atlas with its limit and no file; Borealis owned by Bram.
    async fn empty() -> World {
        let f = Fixture::with_config(|c| {
            c.auth.admins = vec![ADA.to_string()];
            c.safety.write_quota_bytes = SESSION_BUDGET;
            // The purge test removes a trashed file permanently.
            c.safety.allow_hard_delete = true;
        })
        .await;
        f.seed_project(ATLAS, OLIVIA).await;
        f.seed_project(BOREALIS, BRAM).await;
        f.state.admin.add_member(ATLAS, MIA, OLIVIA).await.unwrap();
        f.state.admin.set_quota(ATLAS, Some(2 * MB)).await.unwrap();
        World { f }
    }

    /// The shared setup: Atlas with its seven files.
    async fn shared() -> World {
        let w = World::empty().await;
        for (i, (path, len)) in FILES.iter().enumerate() {
            w.seed(path, *len, i as u64).await;
        }
        w
    }

    /// Seed bytes straight into the volume: setup, not the operation tested.
    async fn seed(&self, path: &str, len: usize, seed: u64) {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        if let Some((parent, _)) = path.rsplit_once('/').filter(|(p, _)| !p.is_empty()) {
            client.makedirs(parent, true).await.unwrap();
        }
        client.write_bytes_atomic(path, &noise(len, seed)).await.unwrap();
    }

    fn as_(&self, person: &str) -> McpServer {
        McpServer::new(self.f.state.clone(), person.to_string())
    }

    async fn read(
        &self,
        person: &str,
        project: &str,
        folder: Option<&str>,
        limit: Option<f64>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = GetProjectFolderUsageArgs {
            project_id: project.into(),
            folder: folder.map(String::from),
            limit,
        };
        self.as_(person).admin_get_project_folder_usage(Parameters(args)).await
    }
}

fn folder_item(path: &str, size: i64, files: i64) -> Value {
    json!({"path": path, "kind": "Folder", "size": size, "file_count": files})
}

fn file_item(path: &str, size: i64) -> Value {
    json!({"path": path, "kind": "File", "size": size})
}

/// The answer of E2E-001: the root of the shared setup.
fn root_answer() -> Value {
    json!({
        "project_id": "Atlas", "max_mb": 2, "used_bytes": 1_225_500,
        "folder": {"path": "/", "size": 1_225_500, "file_count": 7},
        "item_count": 4, "truncated": false,
        "items": [
            folder_item("/video", 800_000, 2),
            folder_item("/data", 300_000, 2),
            folder_item("/docs", 124_000, 2),
            file_item("/readme.md", 1_500),
        ],
    })
}

fn sum_of_items(out: &Value) -> i64 {
    out["items"].as_array().unwrap().iter().map(|e| e["size"].as_i64().unwrap()).sum()
}

/// SPEC-0049/E2E-001: the owner reads the root folder.
#[tokio::test]
async fn spec_0049_e2e_001_the_owner_reads_the_root_folder() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None, None).await);
    assert_eq!(out, root_answer());
    assert_eq!(sum_of_items(&out), 1_225_500);
}

/// SPEC-0049/E2E-002: the admin reads the same root.
#[tokio::test]
async fn spec_0049_e2e_002_the_admin_reads_the_same_root() {
    let w = World::shared().await;
    assert_eq!(ok_json(w.read(ADA, ATLAS, None, None).await), root_answer());
}

/// SPEC-0049/E2E-003: drilling into a folder.
#[tokio::test]
async fn spec_0049_e2e_003_drilling_into_a_folder() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, Some("/video"), None).await);
    assert_eq!(
        out,
        json!({
            "project_id": "Atlas", "max_mb": 2, "used_bytes": 1_225_500,
            "folder": {"path": "/video", "size": 800_000, "file_count": 2},
            "item_count": 2, "truncated": false,
            "items": [file_item("/video/demo.mp4", 600_000), folder_item("/video/clips", 200_000, 1)],
        })
    );
}

/// SPEC-0049/E2E-004: a folder of many small files outranks one bigger file.
#[tokio::test]
async fn spec_0049_e2e_004_a_folder_of_many_small_files_outranks_one_bigger_file() {
    let w = World::empty().await;
    for i in 1..=1000 {
        w.seed(&format!("/logs/l{i:04}.txt"), 100, 1).await;
    }
    w.seed("/big.bin", 90_000, 2).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None, None).await);
    assert_eq!(out["used_bytes"], 190_000);
    assert_eq!(
        out["items"],
        json!([folder_item("/logs", 100_000, 1000), file_item("/big.bin", 90_000)])
    );
    assert_eq!(out["truncated"], false);
}

/// SPEC-0049/E2E-007: a member is refused.
#[tokio::test]
async fn spec_0049_e2e_007_a_member_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(MIA, ATLAS, None, None).await);
    assert!(e.ends_with("'mia@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0049/E2E-008: an outsider is refused.
#[tokio::test]
async fn spec_0049_e2e_008_an_outsider_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None, None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0049/E2E-009: an outsider naming a missing project gets the owner refusal.
#[tokio::test]
async fn spec_0049_e2e_009_an_outsider_naming_a_missing_project_is_refused_the_same_way() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, "Ghost", None, None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Ghost'"), "got: {e}");
}

/// SPEC-0049/E2E-010: the admin naming a missing project.
#[tokio::test]
async fn spec_0049_e2e_010_the_admin_naming_a_missing_project() {
    let w = World::shared().await;
    let e = err_text(w.read(ADA, "Ghost", None, None).await);
    assert!(e.ends_with("project 'Ghost' not found"), "got: {e}");
}

/// SPEC-0049/E2E-011: the owner of another project is refused.
#[tokio::test]
async fn spec_0049_e2e_011_the_owner_of_another_project_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(BRAM, ATLAS, None, None).await);
    assert!(e.ends_with("'bram@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

const CAP_REFUSAL: &str = "limit must be between 1 and 500";

/// SPEC-0049/E2E-012: a cap of 0 is refused.
#[tokio::test]
async fn spec_0049_e2e_012_a_cap_of_zero_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, None, Some(0.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0049/E2E-013: a cap of 501 is refused.
#[tokio::test]
async fn spec_0049_e2e_013_a_cap_of_501_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, None, Some(501.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0049/E2E-014: a negative or fractional cap is refused.
#[tokio::test]
async fn spec_0049_e2e_014_a_negative_or_fractional_cap_is_refused() {
    let w = World::shared().await;
    for bad in [-5.0, 2.5] {
        let e = err_text(w.read(OLIVIA, ATLAS, None, Some(bad)).await);
        assert!(e.ends_with(CAP_REFUSAL), "cap {bad}, got: {e}");
    }
}

/// SPEC-0049/E2E-018: a refusal reveals nothing about the files.
#[tokio::test]
async fn spec_0049_e2e_018_a_refusal_reveals_nothing_about_the_files() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None, None).await);
    for secret in ["video", "readme", "800000", "1225500"] {
        assert!(!e.contains(secret), "the refusal leaks {secret}: {e}");
    }
}

// ── US-0070: name the folder and refuse bad paths ──────────────────────────

/// SPEC-0049/E2E-015: a folder that does not exist (FR-NEW-009).
#[tokio::test]
async fn spec_0049_e2e_015_a_folder_that_does_not_exist() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some("/nothing"), None).await);
    assert!(e.ends_with("folder '/nothing' not found"), "got: {e}");
}

/// SPEC-0049/E2E-016: a file given as the folder (FR-NEW-009).
#[tokio::test]
async fn spec_0049_e2e_016_a_file_given_as_the_folder() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some("/readme.md"), None).await);
    assert!(e.ends_with("'/readme.md' is not a folder"), "got: {e}");
}

/// SPEC-0049/E2E-017: a folder path without a leading slash (FR-NEW-009).
#[tokio::test]
async fn spec_0049_e2e_017_a_folder_path_without_a_leading_slash() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some("video"), None).await);
    assert!(e.ends_with("path must start with /"), "got: {e}");
}

/// SPEC-0049/E2E-027: a trailing slash means the same folder (FR-NEW-009).
#[tokio::test]
async fn spec_0049_e2e_027_a_trailing_slash_means_the_same_folder() {
    let w = World::shared().await;
    let with = ok_json(w.read(OLIVIA, ATLAS, Some("/video/"), None).await);
    let without = ok_json(w.read(OLIVIA, ATLAS, Some("/video"), None).await);
    assert_eq!(with, without);
    assert_eq!(with["folder"]["path"], "/video");
}

/// SPEC-0049/E2E-038: odd folder names are refused as typed (FR-NEW-009).
#[tokio::test]
async fn spec_0049_e2e_038_odd_folder_names_are_refused_as_typed() {
    let w = World::shared().await;
    let cases = [
        ("", "path must start with /"),
        ("/video/../docs", "folder '/video/../docs' not found"),
        ("//video", "folder '//video' not found"),
        ("/video//", "folder '/video//' not found"),
        ("//", "folder '//' not found"),
        ("/Video", "folder '/Video' not found"),
    ];
    for (raw, message) in cases {
        let e = err_text(w.read(OLIVIA, ATLAS, Some(raw), None).await);
        assert!(e.ends_with(message), "folder {raw:?}: got {e}");
    }
    let root = ok_json(w.read(OLIVIA, ATLAS, Some("/"), None).await);
    assert_eq!(root, root_answer());
}

// ── US-0071: the same folder read on the web interface ─────────────────────

/// The real data plane router over the fixture's state, plus a bearer token for
/// `person` (a fresh keypair per call, as in the file list REST tests).
fn rest(f: &Fixture, person: &str) -> (axum::Router, String) {
    let keys = tempfile::tempdir().unwrap();
    let (key, public) = crate::keys::write_keypair(keys.path()).unwrap();
    let mut config = (*f.state.config).clone();
    config.auth.jwt.public_key_path = public.display().to_string();
    let s = &f.state;
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

/// `GET /api/admin/projects/{project}/folder-usage<query>` as `person`.
async fn rest_folder(f: &Fixture, person: &str, project: &str, query: &str) -> (u16, Value) {
    use tower::ServiceExt as _;
    let (app, token) = rest(f, person);
    let req = axum::http::Request::builder()
        .uri(format!("/api/admin/projects/{project}/folder-usage{query}"))
        .header("Authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    // A missing route answers 404 with an empty body: keep that observable as a
    // status assertion failure rather than a parse panic.
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// SPEC-0049/E2E-006: assistants and the web interface agree (FR-NEW-012).
#[tokio::test]
async fn spec_0049_e2e_006_assistants_and_the_web_interface_agree() {
    let w = World::shared().await;
    for (folder, query) in [(None, ""), (Some("/video/"), "?folder=/video/")] {
        let tool = ok_json(w.read(OLIVIA, ATLAS, folder, None).await);
        let (status, body) = rest_folder(&w.f, OLIVIA, ATLAS, query).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body, tool, "folder {folder:?}");
    }
}

/// SPEC-0049/E2E-019: the web interface refuses like the assistants (FR-NEW-008, 012).
#[tokio::test]
async fn spec_0049_e2e_019_the_web_interface_refuses_like_the_assistants() {
    let w = World::shared().await;
    let tool = err_text(w.read(MIA, ATLAS, None, None).await);
    let (status, body) = rest_folder(&w.f, MIA, ATLAS, "").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"], "ERR_FORBIDDEN");
    let detail = body["detail"].as_str().expect("a detail string");
    assert_eq!(detail, "ERR_FORBIDDEN: 'mia@example.com' is not the owner of 'Atlas'");
    assert!(tool.ends_with(detail), "tool: {tool}, rest: {detail}");
}

/// SPEC-0049/E2E-020: the web interface reports a missing folder like the
/// assistants (FR-NEW-009, 012).
#[tokio::test]
async fn spec_0049_e2e_020_the_web_interface_reports_a_missing_folder_like_the_assistants() {
    let w = World::shared().await;
    let tool = err_text(w.read(OLIVIA, ATLAS, Some("/nothing"), None).await);
    let (status, body) = rest_folder(&w.f, OLIVIA, ATLAS, "?folder=/nothing").await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "ERR_NOT_FOUND");
    let detail = body["detail"].as_str().expect("a detail string");
    assert_eq!(detail, "ERR_NOT_FOUND: folder '/nothing' not found");
    assert!(tool.ends_with(detail), "tool: {tool}, rest: {detail}");
}

/// Beyond the acceptance set (DEC-009): `folder=` present and empty is the empty
/// folder text, refused; a cap goes through; a non numeric cap gets the cap message.
#[tokio::test]
async fn spec_0049_us_0071_an_empty_folder_value_is_refused_and_the_cap_goes_through() {
    let w = World::shared().await;
    let (status, body) = rest_folder(&w.f, OLIVIA, ATLAS, "?folder=").await;
    assert_eq!(status, 400, "{body}");
    assert!(body["detail"].as_str().unwrap().ends_with("path must start with /"), "{body}");
    let (status, body) = rest_folder(&w.f, OLIVIA, ATLAS, "?limit=2").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["item_count"], 2);
    assert_eq!(body["truncated"], true);
    let (status, body) = rest_folder(&w.f, OLIVIA, ATLAS, "?limit=abc").await;
    assert_eq!(status, 400, "{body}");
    assert!(body["detail"].as_str().unwrap().ends_with(CAP_REFUSAL), "{body}");
}
