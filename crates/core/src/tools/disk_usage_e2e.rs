//! SPEC-0047 acceptance tests: `admin.get_project_disk_usage`, driven through the
//! real `McpServer` tool method over real storage (SQLite metadata, local
//! blobs). Nothing is mocked.
//!
//! Shared setup: project `Atlas` has a limit of 2 MB and stores five files
//! (975 500 bytes in all); Olivia owns it, Mia is a member, Ada is a platform
//! admin, Otto belongs to nothing, Bram owns `Borealis`.

use crate::docs::DocService;
use crate::docs::service::{Conversion, Manifest, PictureMeta, StubDocService};
use crate::mcp::server::{
    DeleteArgs, ExportZipArgs, GetProjectDiskUsageArgs, GetProjectQuotaArgs, GitCommitArgs,
    McpServer, MountOnlyArgs, SetProjectQuotaArgs, TrashRestoreArgs, WriteBytesArgs,
};
use crate::state::AppState;
use crate::tools::admin::test_support::Fixture;
use base64::Engine as _;
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

const FILES: [(&str, usize); 5] = [
    ("/video/demo.mp4", 600_000),
    ("/data/export.csv", 250_000),
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
            // E2E-005 purges a trashed file permanently.
            c.safety.allow_hard_delete = true;
            c.git.enabled = true;
        })
        .await;
        f.seed_project(ATLAS, OLIVIA).await;
        f.seed_project(BOREALIS, BRAM).await;
        f.state.admin.add_member(ATLAS, MIA, OLIVIA).await.unwrap();
        f.state.admin.set_quota(ATLAS, Some(2 * MB)).await.unwrap();
        World { f }
    }

    /// The shared setup: Atlas with its five files.
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

    /// [`Self::as_`] with `doc` as the document service, sharing the fixture's stores.
    fn as_with_doc(&self, person: &str, doc: Arc<dyn DocService>) -> McpServer {
        let s = &self.f.state;
        let state = AppState {
            config: s.config.clone(),
            admin: s.admin.clone(),
            stores: s.stores.clone(),
            safety: s.safety.clone(),
            identity: s.identity.clone(),
            editors: s.editors.clone(),
            doc_service: Some(doc),
            search: None,
        };
        McpServer::new(Arc::new(state), person.to_string())
    }

    async fn delete(&self, person: &str, path: &str, trash: bool) -> Value {
        let args =
            DeleteArgs { mount_id: ATLAS.into(), path: path.into(), recursive: false, trash };
        ok_json(self.as_(person).fs_delete(Parameters(args)).await)
    }

    /// The path under which `name` sits in the trash.
    async fn trashed(&self, name: &str) -> String {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        let hits: Vec<String> = client
            .meta
            .subtree("/")
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.is_file() && n.path.starts_with("/.mcp_trash/") && n.path.contains(name))
            .map(|n| n.path)
            .collect();
        assert_eq!(hits.len(), 1, "one trashed {name}: {hits:?}");
        hits[0].clone()
    }

    async fn read(
        &self,
        person: &str,
        project: &str,
        limit: Option<f64>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = GetProjectDiskUsageArgs { project_id: project.into(), limit };
        self.as_(person).admin_get_project_disk_usage(Parameters(args)).await
    }
}

fn entry(path: &str, size: i64) -> Value {
    json!({"path": path, "size": size, "state": "Active", "original_path": null})
}

/// The standard answer of the spec: five entries, each `Active`.
fn standard() -> Value {
    json!({
        "project_id": "Atlas",
        "max_mb": 2,
        "used_bytes": 975_500,
        "file_count": 5,
        "truncated": false,
        "entries": [
            entry("/video/demo.mp4", 600_000),
            entry("/data/export.csv", 250_000),
            entry("/docs/spec.docx", 120_000),
            entry("/docs/notes.txt", 4_000),
            entry("/readme.md", 1_500),
        ],
    })
}

/// SPEC-0047/E2E-001: the owner reads the breakdown (FR-NEW-001, 002, 003, 008).
#[tokio::test]
async fn spec_0047_e2e_001_the_owner_reads_the_breakdown() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out, standard());
    let sum: i64 =
        out["entries"].as_array().unwrap().iter().map(|e| e["size"].as_i64().unwrap()).sum();
    assert_eq!(sum, out["used_bytes"].as_i64().unwrap());
}

/// SPEC-0047/E2E-002: the admin reads the same breakdown (FR-NEW-001, 007).
#[tokio::test]
async fn spec_0047_e2e_002_the_admin_reads_the_same_breakdown() {
    let w = World::shared().await;
    assert_eq!(ok_json(w.read(ADA, ATLAS, None).await), standard());
}

/// SPEC-0047/E2E-007: a member is refused (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_007_a_member_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(MIA, ATLAS, None).await);
    assert!(e.ends_with("'mia@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0047/E2E-008: an outsider is refused (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_008_an_outsider_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0047/E2E-009: an outsider naming a missing project gets the owner
/// refusal, so existence cannot be probed (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_009_an_outsider_naming_a_missing_project_is_refused_the_same_way() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, "Ghost", None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Ghost'"), "got: {e}");
}

/// SPEC-0047/E2E-010: the admin naming a missing project (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_010_the_admin_naming_a_missing_project() {
    let w = World::shared().await;
    let e = err_text(w.read(ADA, "Ghost", None).await);
    assert!(e.ends_with("project 'Ghost' not found"), "got: {e}");
}

/// SPEC-0047/E2E-011: the owner of another project is refused (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_011_the_owner_of_another_project_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(BRAM, ATLAS, None).await);
    assert!(e.ends_with("'bram@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

const CAP_REFUSAL: &str = "limit must be between 1 and 500";

/// SPEC-0047/E2E-012: a cap of 0 is refused (FR-NEW-004).
#[tokio::test]
async fn spec_0047_e2e_012_a_cap_of_zero_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some(0.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0047/E2E-013: a cap of 501 is refused (FR-NEW-004).
#[tokio::test]
async fn spec_0047_e2e_013_a_cap_of_501_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some(501.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0047/E2E-014: a negative or fractional cap is refused (FR-NEW-004).
#[tokio::test]
async fn spec_0047_e2e_014_a_negative_or_fractional_cap_is_refused() {
    let w = World::shared().await;
    for bad in [-5.0, 2.5] {
        let e = err_text(w.read(OLIVIA, ATLAS, Some(bad)).await);
        assert!(e.ends_with(CAP_REFUSAL), "cap {bad}, got: {e}");
    }
}

/// SPEC-0047/E2E-016: a refusal reveals nothing about the files (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_016_a_refusal_reveals_nothing_about_the_files() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None).await);
    for secret in ["demo", "export", "600000", "975500", "readme"] {
        assert!(!e.contains(secret), "the refusal leaks {secret}: {e}");
    }
}

/// SPEC-0047/E2E-022: an empty project (FR-NEW-001, 003, 005).
#[tokio::test]
async fn spec_0047_e2e_022_an_empty_project() {
    let w = World::empty().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(
        out,
        json!({"project_id": "Atlas", "max_mb": 2, "used_bytes": 0, "file_count": 0,
               "truncated": false, "entries": []})
    );
}

/// SPEC-0047/E2E-023: the default cap is 100 (FR-NEW-004, 005).
#[tokio::test]
async fn spec_0047_e2e_023_the_default_cap_is_100() {
    let w = World::empty().await;
    for i in 0..120 {
        w.seed(&format!("/f{i:03}.bin"), 1_000, i).await;
    }
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out["entries"].as_array().unwrap().len(), 100);
    assert_eq!(out["file_count"], 120);
    assert_eq!(out["used_bytes"], 120_000);
    assert_eq!(out["truncated"], true);
}

/// SPEC-0047/E2E-027: equal sizes are ordered by path in character code order,
/// so an uppercase name comes before a lowercase one (FR-NEW-002).
#[tokio::test]
async fn spec_0047_e2e_027_equal_sizes_are_ordered_by_path() {
    let w = World::empty().await;
    for (i, p) in ["/b.txt", "/a.txt", "/C.txt"].iter().enumerate() {
        w.seed(p, 10, i as u64).await;
    }
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let paths: Vec<&str> =
        out["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["/C.txt", "/a.txt", "/b.txt"]);
}

fn trash_entry_of(out: &Value, size: i64) -> Value {
    out["entries"].as_array().unwrap().iter().find(|e| e["size"] == size).cloned().expect("entry")
}

/// SPEC-0047/E2E-003: a trashed file is listed as in the trash (FR-NEW-003, 006).
#[tokio::test]
async fn spec_0047_e2e_003_a_trashed_file_is_listed_as_in_the_trash() {
    let w = World::shared().await;
    w.delete(MIA, "/docs/notes.txt", true).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let e = trash_entry_of(&out, 4_000);
    assert_eq!(e["state"], "In trash");
    assert_eq!(e["original_path"], "/docs/notes.txt");
    assert!(e["path"].as_str().unwrap().starts_with("/.mcp_trash/"), "got: {e}");
    for other in [600_000, 250_000, 120_000, 1_500] {
        assert_eq!(trash_entry_of(&out, other)["state"], "Active");
    }
    assert_eq!((out["file_count"].clone(), out["used_bytes"].clone()), (json!(5), json!(975_500)));
    assert_eq!(out["truncated"], false);
}

/// SPEC-0047/E2E-005: a purged file disappears (FR-NEW-001, 006).
#[tokio::test]
async fn spec_0047_e2e_005_a_purged_file_disappears() {
    let w = World::shared().await;
    w.delete(MIA, "/docs/notes.txt", true).await;
    let trashed = w.trashed("notes.txt").await;
    w.delete(MIA, &trashed, false).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out["file_count"], 4);
    assert_eq!(out["used_bytes"], 971_500);
    assert!(out["entries"].as_array().unwrap().iter().all(|e| e["size"] != 4_000), "got: {out}");
    assert_eq!(out["truncated"], false);
}

/// SPEC-0047/E2E-006: the sizes add up to the usage (FR-NEW-006, 010).
#[tokio::test]
async fn spec_0047_e2e_006_the_sizes_add_up_to_the_usage() {
    let w = World::empty().await;
    for i in 0..120 {
        w.seed(&format!("/f{i:03}.bin"), 1_000, i).await;
    }
    let out = ok_json(w.read(OLIVIA, ATLAS, Some(20.0)).await);
    assert_eq!(out["used_bytes"], 120_000);
    assert_eq!(out["file_count"], 120);
    assert_eq!(out["entries"].as_array().unwrap().len(), 20);
    assert_eq!(out["truncated"], true);
    let all = ok_json(w.read(OLIVIA, ATLAS, Some(500.0)).await);
    let entries = all["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 120);
    assert_eq!(entries.iter().map(|e| e["size"].as_i64().unwrap()).sum::<i64>(), 120_000);
}

/// SPEC-0047/E2E-019: deleting moves a file to the trash (FR-NEW-003, 006).
#[tokio::test]
async fn spec_0047_e2e_019_deleting_moves_a_file_to_the_trash() {
    let w = World::shared().await;
    w.delete(MIA, "/data/export.csv", true).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!((out["file_count"].clone(), out["used_bytes"].clone()), (json!(5), json!(975_500)));
    let second = &out["entries"][1];
    assert_eq!(second["size"], 250_000);
    assert_eq!(second["state"], "In trash");
    assert_eq!(second["original_path"], "/data/export.csv");
    let sizes: Vec<i64> =
        out["entries"].as_array().unwrap().iter().map(|e| e["size"].as_i64().unwrap()).collect();
    assert_eq!(sizes, [600_000, 250_000, 120_000, 4_000, 1_500]);
}

/// SPEC-0047/E2E-020: restoring brings a file back to Active (FR-NEW-003).
#[tokio::test]
async fn spec_0047_e2e_020_restoring_brings_a_file_back_to_active() {
    let w = World::shared().await;
    w.delete(MIA, "/data/export.csv", true).await;
    let trashed = w.trashed("export.csv").await;
    let args = TrashRestoreArgs { mount_id: ATLAS.into(), trash_path: trashed };
    ok_json(w.as_(MIA).fs_trash_restore(Parameters(args)).await);
    assert_eq!(ok_json(w.read(OLIVIA, ATLAS, None).await), standard());
}

/// SPEC-0047/E2E-029: files sharing content each show their full size (FR-NEW-006).
#[tokio::test]
async fn spec_0047_e2e_029_files_sharing_content_each_show_their_full_size() {
    let w = World::empty().await;
    w.seed("/a/copy1.bin", 100_000, 7).await;
    w.seed("/a/copy2.bin", 100_000, 7).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out["used_bytes"], 200_000);
    assert_eq!(
        out["entries"],
        json!([entry("/a/copy1.bin", 100_000), entry("/a/copy2.bin", 100_000)])
    );
}

/// SPEC-0047/E2E-031: history, exports and artifacts are not counted (FR-NEW-006).
#[tokio::test]
async fn spec_0047_e2e_031_history_exports_and_artifacts_are_not_counted() {
    let w = World::empty().await;
    w.seed("/a.txt", 10_000, 3).await;
    // History: a commit of the project.
    let store = Arc::new(crate::git::GitRepoStore::new(
        w.f.state.config.clone(),
        w.f.state.stores.relational().clone(),
    ));
    let ctx = w.f.ctx(OLIVIA);
    crate::tools::git::tool_init(
        ctx.clone(),
        MountOnlyArgs { mount_id: ATLAS.into() },
        Some(store.clone()),
    )
    .await
    .unwrap();
    let commit = GitCommitArgs {
        mount_id: ATLAS.into(),
        message: "all".into(),
        author_name: None,
        author_email: None,
    };
    crate::tools::git::tool_commit(ctx, commit, Some(store)).await.unwrap();
    // An export zip.
    let zip = ExportZipArgs { mount_id: ATLAS.into(), paths: vec!["/a.txt".into()] };
    ok_json(w.as_(OLIVIA).fs_export_zip(Parameters(zip)).await);
    // A conversion keeping two images and a table as artifacts.
    let pics: Vec<(String, Vec<u8>, String)> = (1..=2)
        .map(|n| (format!("figures/f{n}.png"), noise(3_000, n), format!("Figure {n}")))
        .collect();
    let md = "![a](figures/f1.png)\n\n![b](figures/f2.png)\n\n| H | V |\n| - | - |\n| x | 1 |\n"
        .to_string();
    let files = pics.iter().map(|(t, b, _)| (t.clone(), b.clone())).collect();
    let pictures = pics
        .into_iter()
        .map(|(t, _, c)| (t, PictureMeta { caption: c, caption_failed: false, page: None }))
        .collect();
    let doc: Arc<dyn DocService> = Arc::new(StubDocService::bundle(Conversion {
        markdown: md,
        files,
        manifest: Some(Manifest { pictures }),
    }));
    let upload = WriteBytesArgs {
        mount_id: ATLAS.into(),
        path: "/e.pdf".into(),
        base64: base64::engine::general_purpose::STANDARD.encode(noise(5_000, 9)),
        overwrite: false,
        create_parents: true,
        trigger_documentation_service: true,
    };
    ok_json(w.as_with_doc(OLIVIA, doc).fs_write_bytes(Parameters(upload)).await);
    // Only project files are listed: the pdf, its markdown, the text file.
    let mid = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let paths: Vec<&str> =
        mid["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(mid["file_count"], 3, "got: {mid}");
    assert!(paths.iter().all(|p| !p.contains("figures")), "got: {paths:?}");
    // Remove the conversion's two files for good: the artifacts stay behind.
    w.delete(OLIVIA, "/e.pdf", false).await;
    w.delete(OLIVIA, "/e.md", false).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out["file_count"], 1);
    assert_eq!(out["used_bytes"], 10_000);
    assert_eq!(out["entries"], json!([entry("/a.txt", 10_000)]));
}

/// Beyond the acceptance set: a file inside a trashed folder is `In trash` and
/// shows the path it had inside that folder (DEC-008).
#[tokio::test]
async fn spec_0047_us_0065_a_file_in_a_trashed_folder_shows_its_original_path() {
    let w = World::shared().await;
    let args =
        DeleteArgs { mount_id: ATLAS.into(), path: "/docs".into(), recursive: true, trash: true };
    ok_json(w.as_(MIA).fs_delete(Parameters(args)).await);
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let spec = trash_entry_of(&out, 120_000);
    assert_eq!(spec["state"], "In trash");
    assert_eq!(spec["original_path"], "/docs/spec.docx");
    let notes = trash_entry_of(&out, 4_000);
    assert_eq!(notes["original_path"], "/docs/notes.txt");
    assert_eq!(out["file_count"], 5);
    assert_eq!(out["used_bytes"], 975_500);
}

// ── US-0066: the same breakdown on the web interface ────────────────────────

/// The real data plane router over the fixture's state, plus a bearer token for
/// `person` (a fresh keypair per call, as in the diff and quota REST tests).
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

/// `GET /api/admin/projects/{project}/disk-usage<query>` as `person`.
async fn rest_usage(f: &Fixture, person: &str, project: &str, query: &str) -> (u16, Value) {
    use tower::ServiceExt as _;
    let (app, token) = rest(f, person);
    let req = axum::http::Request::builder()
        .uri(format!("/api/admin/projects/{project}/disk-usage{query}"))
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

/// SPEC-0047/E2E-004: assistants and the web interface agree (FR-NEW-011).
#[tokio::test]
async fn spec_0047_e2e_004_assistants_and_the_web_interface_agree() {
    let w = World::shared().await;
    let tool = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let (status, body) = rest_usage(&w.f, OLIVIA, ATLAS, "").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, tool);
    assert_eq!(body, standard());
}

/// SPEC-0047/E2E-015: the web interface refuses like the assistants
/// (FR-NEW-007, 011).
#[tokio::test]
async fn spec_0047_e2e_015_the_web_interface_refuses_like_the_assistants() {
    let w = World::shared().await;
    let tool = err_text(w.read(MIA, ATLAS, None).await);
    let (status, body) = rest_usage(&w.f, MIA, ATLAS, "").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"], "ERR_FORBIDDEN");
    let detail = body["detail"].as_str().expect("a detail string");
    assert_eq!(detail, "ERR_FORBIDDEN: 'mia@example.com' is not the owner of 'Atlas'");
    assert!(tool.ends_with(detail), "tool: {tool}, rest: {detail}");
}

/// SPEC-0047/E2E-034: the cap through the web interface (FR-NEW-004, 011).
#[tokio::test]
async fn spec_0047_e2e_034_the_cap_through_the_web_interface() {
    let w = World::shared().await;
    let (status, body) = rest_usage(&w.f, OLIVIA, ATLAS, "?limit=2").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["entries"],
        json!([entry("/video/demo.mp4", 600_000), entry("/data/export.csv", 250_000)])
    );
    assert_eq!(body["file_count"], 5);
    assert_eq!(body["truncated"], true);
    let (status, body) = rest_usage(&w.f, OLIVIA, ATLAS, "?limit=0").await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "ERR_INVALID_ARGUMENT");
    assert!(body["detail"].as_str().unwrap().ends_with(CAP_REFUSAL), "{body}");
    let (status, body) = rest_usage(&w.f, OLIVIA, ATLAS, "?limit=abc").await;
    assert_eq!(status, 400, "{body}");
    assert!(body["detail"].as_str().unwrap().ends_with(CAP_REFUSAL), "{body}");
}

/// SPEC-0047/E2E-035: the default cap through the web interface (FR-NEW-004, 005, 011).
#[tokio::test]
async fn spec_0047_e2e_035_the_default_cap_through_the_web_interface() {
    let w = World::empty().await;
    for i in 0..120 {
        w.seed(&format!("/f{i:03}.bin"), 1_000, i).await;
    }
    let tool = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let (status, body) = rest_usage(&w.f, OLIVIA, ATLAS, "").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["entries"].as_array().unwrap().len(), 100);
    assert_eq!(body["file_count"], 120);
    assert_eq!(body["truncated"], true);
    assert_eq!(body, tool);
}

// ── US-0067: one moment, read only, hardening ──────────────────────────────

fn paths_of(out: &Value) -> Vec<String> {
    out["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap().into()).collect()
}

async fn write_bytes(
    w: &World,
    person: &str,
    path: &str,
    len: usize,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let args = WriteBytesArgs {
        mount_id: ATLAS.into(),
        path: path.into(),
        base64: base64::engine::general_purpose::STANDARD.encode(noise(len, 5)),
        overwrite: false,
        create_parents: true,
        trigger_documentation_service: false,
    };
    w.as_(person).fs_write_bytes(Parameters(args)).await
}

/// SPEC-0047/E2E-017: reading changes nothing (FR-NEW-009).
#[tokio::test]
async fn spec_0047_e2e_017_reading_changes_nothing() {
    let w = World::shared().await;
    ok_json(write_bytes(&w, OLIVIA, "/session.bin", 100_000).await);
    let before = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let used_before = before["used_bytes"].clone();
    for _ in 0..2 {
        assert_eq!(ok_json(w.read(OLIVIA, ATLAS, None).await), before);
    }
    let quota = ok_json(
        w.as_(OLIVIA)
            .admin_get_project_quota(Parameters(GetProjectQuotaArgs { project_id: ATLAS.into() }))
            .await,
    );
    assert_eq!(quota["used_bytes"], used_before);
    assert_eq!(before["file_count"], 6);
    // The session budget (262 144) still has 162 144 bytes: one more is refused,
    // 162 144 are accepted.
    let refused = err_text(write_bytes(&w, OLIVIA, "/over.bin", 162_145).await);
    assert!(refused.ends_with("session write quota of 262144 bytes exceeded"), "got: {refused}");
    ok_json(write_bytes(&w, OLIVIA, "/exact.bin", 162_144).await);
}

/// SPEC-0047/E2E-018: a project over its limit is readable (FR-NEW-008, 009).
#[tokio::test]
async fn spec_0047_e2e_018_a_project_over_its_limit_is_readable() {
    let w = World::empty().await;
    for (i, (p, len)) in
        [("/a.bin", 700_000), ("/b.bin", 500_000), ("/c.bin", 300_000)].iter().enumerate()
    {
        w.seed(p, *len, i as u64).await;
    }
    let lower = SetProjectQuotaArgs { project_id: ATLAS.into(), max_mb: Some(1) };
    ok_json(w.as_(ADA).admin_set_project_quota(Parameters(lower)).await);
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(
        out,
        json!({"project_id": "Atlas", "max_mb": 1, "used_bytes": 1_500_000, "file_count": 3,
               "truncated": false,
               "entries": [entry("/a.bin", 700_000), entry("/b.bin", 500_000), entry("/c.bin", 300_000)]})
    );
}

/// SPEC-0047/E2E-021: an unlimited project (FR-NEW-008).
#[tokio::test]
async fn spec_0047_e2e_021_an_unlimited_project() {
    let w = World::shared().await;
    let clear = SetProjectQuotaArgs { project_id: ATLAS.into(), max_mb: None };
    ok_json(w.as_(ADA).admin_set_project_quota(Parameters(clear)).await);
    let out = ok_json(w.read(ADA, ATLAS, None).await);
    assert!(out["max_mb"].is_null(), "neither 0 nor text: {out}");
    assert_eq!(out["used_bytes"], 975_500);
    assert_eq!(out["entries"].as_array().unwrap().len(), 5);
}

async fn world_with_120_files() -> World {
    let w = World::empty().await;
    for i in 0..120 {
        w.seed(&format!("/f{i:03}.bin"), 1_000, i).await;
    }
    w
}

/// SPEC-0047/E2E-024: a cap equal to the number of files (FR-NEW-004, 005).
#[tokio::test]
async fn spec_0047_e2e_024_a_cap_equal_to_the_number_of_files() {
    let w = world_with_120_files().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, Some(120.0)).await);
    assert_eq!(out["entries"].as_array().unwrap().len(), 120);
    assert_eq!(out["truncated"], false);
}

/// SPEC-0047/E2E-025: a cap one below the number of files (FR-NEW-004, 005).
#[tokio::test]
async fn spec_0047_e2e_025_a_cap_one_below_the_number_of_files() {
    let w = world_with_120_files().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, Some(119.0)).await);
    assert_eq!(out["entries"].as_array().unwrap().len(), 119);
    assert_eq!(out["file_count"], 120);
    assert_eq!(out["truncated"], true);
}

/// SPEC-0047/E2E-026: the smallest and the largest accepted caps (FR-NEW-001, 004, 005).
#[tokio::test]
async fn spec_0047_e2e_026_the_smallest_and_the_largest_accepted_caps() {
    let w = World::shared().await;
    let one = ok_json(w.read(OLIVIA, ATLAS, Some(1.0)).await);
    assert_eq!(one["entries"], json!([entry("/video/demo.mp4", 600_000)]));
    assert_eq!(one["file_count"], 5);
    assert_eq!(one["truncated"], true);
    let max = ok_json(w.read(OLIVIA, ATLAS, Some(500.0)).await);
    assert_eq!(max, standard());
}

/// SPEC-0047/E2E-028: empty files come last (FR-NEW-002).
#[tokio::test]
async fn spec_0047_e2e_028_empty_files_come_last() {
    let w = World::empty().await;
    w.seed("/empty.txt", 0, 1).await;
    w.seed("/a.txt", 10, 2).await;
    w.seed("/b.txt", 20, 3).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(
        out["entries"],
        json!([entry("/b.txt", 20), entry("/a.txt", 10), entry("/empty.txt", 0)])
    );
}

/// SPEC-0047/E2E-030: accents and emoji stay intact (FR-NEW-003).
#[tokio::test]
async fn spec_0047_e2e_030_accents_and_emoji_stay_intact() {
    let w = World::empty().await;
    w.seed("/rapports/été 📊.xlsx", 5_000, 4).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out["entries"], json!([entry("/rapports/été 📊.xlsx", 5_000)]));
}

/// SPEC-0047/E2E-033: folders are not listed (FR-NEW-003).
#[tokio::test]
async fn spec_0047_e2e_033_folders_are_not_listed() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let listed = paths_of(&out);
    for folder in ["/video", "/data", "/docs"] {
        assert!(!listed.iter().any(|p| p == folder), "{folder} listed: {listed:?}");
    }
    assert_eq!(out["file_count"], 5);
}

/// SPEC-0047/E2E-032: a write during the read is seen entirely or not at all
/// (FR-NEW-009, 010). Deterministic, no sleep: the test plays the writer by
/// holding the project's admission lock, as every write does for its whole
/// duration; the read must wait for it.
#[tokio::test]
async fn spec_0047_e2e_032_a_write_during_the_read_is_seen_entirely_or_not_at_all() {
    let w = World::shared().await;
    let client = w.f.state.stores.client(ATLAS).await.unwrap();
    let writer = client.admission.clone().lock_owned().await;

    let server = w.as_(OLIVIA);
    let read = tokio::spawn(async move {
        let args = GetProjectDiskUsageArgs { project_id: ATLAS.into(), limit: None };
        server.admin_get_project_disk_usage(Parameters(args)).await
    });
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(!read.is_finished(), "the read must wait for the write in flight");

    client.write_bytes_atomic("/docs/new.txt", &noise(30_000, 11)).await.unwrap();
    drop(writer);

    let out = ok_json(read.await.unwrap());
    assert_eq!(out["file_count"], 6, "{out}");
    assert_eq!(out["used_bytes"], 1_005_500);
    let sum: i64 =
        out["entries"].as_array().unwrap().iter().map(|e| e["size"].as_i64().unwrap()).sum();
    assert_eq!(sum, 1_005_500);
    assert_eq!(ok_json(w.read(OLIVIA, ATLAS, None).await)["file_count"], 6);
}
