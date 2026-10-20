//! SPEC-0045 US-0055 acceptance tests: `fs.diff_files`, driven through the real
//! `McpServer::fs_diff_files` handler over real storage (SQLite metadata, local
//! blobs). Nothing is mocked.

use crate::mcp::server::{DiffFilesArgs, EditArgs, McpServer, MultiEditArgs, ReadArgs, WriteArgs};
use crate::storage::traits::AdminBackend;
use crate::tools::admin::test_support::Fixture;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};

const OWNER: &str = "owner@test.com";
const ALICE: &str = "alice@example.com";
const BOB: &str = "bob@example.com";
const CAROL: &str = "carol@example.com";

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

/// The common data: alpha and beta, Alice in both, Bob in alpha only, and the
/// two notes of alpha.
async fn world() -> Fixture {
    let f = Fixture::new().await;
    f.seed_project("alpha", OWNER).await;
    f.seed_project("beta", OWNER).await;
    f.state.admin.add_member("alpha", ALICE, OWNER).await.unwrap();
    f.state.admin.add_member("beta", ALICE, OWNER).await.unwrap();
    f.state.admin.add_member("alpha", BOB, OWNER).await.unwrap();
    put(&f, "alpha", "/notes/a.txt", "one\ntwo\nthree\n").await;
    put(&f, "alpha", "/notes/b.txt", "one\n2\nthree\n").await;
    f
}

async fn put(f: &Fixture, project: &str, path: &str, text: &str) {
    f.state.stores.client(project).await.unwrap().write_text_atomic(path, text).await.unwrap();
}

async fn diff(
    f: &Fixture,
    person: &str,
    a: (&str, &str),
    b: (&str, &str),
) -> Result<CallToolResult, rmcp::ErrorData> {
    diff_with(f, person, a, b, None).await
}

async fn diff_with(
    f: &Fixture,
    person: &str,
    a: (&str, &str),
    b: (&str, &str),
    context_lines: Option<Value>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let server = McpServer::new(f.state.clone(), person.to_string());
    server
        .fs_diff_files(Parameters(DiffFilesArgs {
            mount_id_a: a.0.to_string(),
            path_a: a.1.to_string(),
            mount_id_b: b.0.to_string(),
            path_b: b.1.to_string(),
            context_lines,
        }))
        .await
}

/// SPEC-0045/E2E-001: two files of one project differ on one line.
#[tokio::test]
async fn spec_0045_e2e_001_two_files_of_one_project() {
    let f = world().await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    assert_eq!(
        v,
        json!({
            "identical": false,
            "diff": "--- alpha:/notes/a.txt\n+++ alpha:/notes/b.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+2\n three\n"
        })
    );
}

/// SPEC-0045/E2E-002: two projects.
#[tokio::test]
async fn spec_0045_e2e_002_two_projects() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert_eq!(v["identical"], false);
    assert_eq!(
        v["diff"],
        "--- alpha:/cfg/app.txt\n+++ beta:/cfg/app.txt\n@@ -1 +1 @@\n-port=80\n+port=8080\n"
    );
}

/// SPEC-0045/E2E-003: identical files.
#[tokio::test]
async fn spec_0045_e2e_003_identical_files() {
    let f = world().await;
    put(&f, "alpha", "/notes/copy.txt", "one\ntwo\nthree\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/copy.txt")).await);
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

/// SPEC-0045/E2E-107: no access to the second project, the answer is the same
/// whether the file exists or not.
#[tokio::test]
async fn spec_0045_e2e_107_no_access_to_the_second_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "secret=1\n").await;
    let e1 = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/cfg/app.txt")).await);
    assert!(
        e1.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"),
        "got: {e1}"
    );
    for leak in ["secret", "one", "two", "three"] {
        assert!(!e1.contains(leak), "leaked {leak}: {e1}");
    }
    let e2 = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/cfg/nothing.txt")).await);
    assert_eq!(e1, e2);
}

/// SPEC-0045/E2E-108: no access to the first project.
#[tokio::test]
async fn spec_0045_e2e_108_no_access_to_the_first_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "secret=1\n").await;
    let e = err_text(diff(&f, BOB, ("beta", "/cfg/app.txt"), ("alpha", "/notes/a.txt")).await);
    assert!(e.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"), "got: {e}");
    for leak in ["secret", "one", "two", "three"] {
        assert!(!e.contains(leak), "leaked {leak}: {e}");
    }
}

/// SPEC-0045/E2E-201: empty against non empty.
#[tokio::test]
async fn spec_0045_e2e_201_empty_against_non_empty() {
    let f = world().await;
    put(&f, "alpha", "/e.txt", "").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/e.txt"), ("alpha", "/notes/b.txt")).await);
    assert_eq!(v["identical"], false);
    assert_eq!(
        v["diff"],
        "--- alpha:/e.txt\n+++ alpha:/notes/b.txt\n@@ -0,0 +1,3 @@\n+one\n+2\n+three\n"
    );
}

/// SPEC-0045/E2E-202: two empty files.
#[tokio::test]
async fn spec_0045_e2e_202_two_empty_files() {
    let f = world().await;
    put(&f, "alpha", "/e1.txt", "").await;
    put(&f, "alpha", "/e2.txt", "").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/e1.txt"), ("alpha", "/e2.txt")).await);
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

/// SPEC-0045/E2E-203: the same file on both sides changes nothing.
#[tokio::test]
async fn spec_0045_e2e_203_the_same_file_on_both_sides() {
    let f = world().await;
    let client = f.state.stores.client("alpha").await.unwrap();
    let before_text = client.read_text("/notes/a.txt").await.unwrap();
    let before_used = crate::quota::used_bytes(&client).await.unwrap();
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/a.txt")).await);
    assert_eq!(v, json!({"identical": true, "diff": ""}));
    assert_eq!(client.read_text("/notes/a.txt").await.unwrap(), before_text);
    assert_eq!(crate::quota::used_bytes(&client).await.unwrap(), before_used);
}

/// SPEC-0045/E2E-206: Windows against Unix line endings.
#[tokio::test]
async fn spec_0045_e2e_206_windows_against_unix_line_endings() {
    let f = world().await;
    put(&f, "alpha", "/w.txt", "one\r\ntwo\r\n").await;
    put(&f, "alpha", "/u.txt", "one\ntwo\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/w.txt"), ("alpha", "/u.txt")).await);
    assert_eq!(v["identical"], false);
    let d = v["diff"].as_str().unwrap();
    let body: Vec<&str> = d.lines().skip(2).collect();
    assert_eq!(body.iter().filter(|l| l.starts_with('-')).count(), 2, "got: {d}");
    assert_eq!(body.iter().filter(|l| l.starts_with('+')).count(), 2, "got: {d}");
}

/// SPEC-0045/E2E-207: no final line ending, and no "no newline" marker.
#[tokio::test]
async fn spec_0045_e2e_207_no_final_line_ending() {
    let f = world().await;
    put(&f, "alpha", "/n1.txt", "a").await;
    put(&f, "alpha", "/n2.txt", "b").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/n1.txt"), ("alpha", "/n2.txt")).await);
    assert_eq!(v["identical"], false);
    assert_eq!(v["diff"], "--- alpha:/n1.txt\n+++ alpha:/n2.txt\n@@ -1 +1 @@\n-a\n+b\n");
}

/// SPEC-0045/E2E-208: two distant changes make two blocks.
#[tokio::test]
async fn spec_0045_e2e_208_two_distant_changes() {
    let f = world().await;
    let old: String = (1..=20).map(|i| format!("line{i}\n")).collect();
    let new = old.replacen("line1\n", "LINE1\n", 1).replacen("line20\n", "LINE20\n", 1);
    put(&f, "alpha", "/l1.txt", &old).await;
    put(&f, "alpha", "/l2.txt", &new).await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/l1.txt"), ("alpha", "/l2.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/l1.txt\n+++ alpha:/l2.txt\n\
         @@ -1,4 +1,4 @@\n-line1\n+LINE1\n line2\n line3\n line4\n\
         @@ -17,4 +17,4 @@\n line17\n line18\n line19\n-line20\n+LINE20\n"
    );
}

/// SPEC-0045/E2E-209: accents and emoji survive in headers and lines.
#[tokio::test]
async fn spec_0045_e2e_209_accents_and_emoji() {
    let f = world().await;
    put(&f, "alpha", "/é/a.txt", "café ☕\n").await;
    put(&f, "alpha", "/é/b.txt", "café 🍵\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/é/a.txt"), ("alpha", "/é/b.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/é/a.txt\n+++ alpha:/é/b.txt\n@@ -1 +1 @@\n-café ☕\n+café 🍵\n"
    );
}

/// SPEC-0045/E2E-210: more than 2000 lines, no truncation.
#[tokio::test]
async fn spec_0045_e2e_210_more_than_2000_lines() {
    let f = world().await;
    let old: String = (1..=3000).map(|i| format!("row{i}\n")).collect();
    let new = old.replacen("row2999\n", "CHANGED\n", 1);
    put(&f, "alpha", "/big1.txt", &old).await;
    put(&f, "alpha", "/big2.txt", &new).await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/big1.txt"), ("alpha", "/big2.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/big1.txt\n+++ alpha:/big2.txt\n@@ -2996,5 +2996,5 @@\n row2996\n row2997\n row2998\n-row2999\n+CHANGED\n row3000\n"
    );
}

/// SPEC-0045/E2E-211: a plain member (not owner) of both projects.
#[tokio::test]
async fn spec_0045_e2e_211_a_plain_member() {
    let f = world().await;
    f.state.admin.add_member("alpha", CAROL, OWNER).await.unwrap();
    f.state.admin.add_member("beta", CAROL, OWNER).await.unwrap();
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    let carol = ok_json(diff(&f, CAROL, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    let alice = ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert_eq!(carol, alice);
    assert_eq!(carol["identical"], false);
}

/// SPEC-0045/E2E-225: doubled separators are normalized, in errors and headers.
#[tokio::test]
async fn spec_0045_e2e_225_path_with_doubled_separators() {
    let f = world().await;
    let e = err_text(
        diff(&f, ALICE, ("alpha", "/notes//a.txt"), ("alpha", "/notes/missing.txt")).await,
    );
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found"), "got: {e}");
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes//a.txt"), ("alpha", "/notes/b.txt")).await);
    assert!(v["diff"].as_str().unwrap().starts_with("--- alpha:/notes/a.txt\n"), "got: {v}");
}

/// SPEC-0045/E2E-101: first file missing, named with its project.
#[tokio::test]
async fn spec_0045_e2e_101_first_file_missing() {
    let f = world().await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/notes/missing.txt"), ("alpha", "/notes/b.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found"), "got: {e}");
    assert!(!e.contains("identical") && !e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-102: second file missing, the second one is named.
#[tokio::test]
async fn spec_0045_e2e_102_second_file_missing() {
    let f = world().await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/missing.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found"), "got: {e}");
    assert!(!e.contains("identical") && !e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-103: a directory is refused, named with its project.
#[tokio::test]
async fn spec_0045_e2e_103_a_directory() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes"), ("alpha", "/notes/b.txt")).await);
    assert!(e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory"), "got: {e}");
}

/// SPEC-0045/E2E-109: deleted project (green on arrival, inherited).
#[tokio::test]
async fn spec_0045_e2e_109_deleted_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "x\n").await;
    f.admin_store.soft_delete_project("beta").await.unwrap();
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("beta", "/cfg/app.txt")).await);
    assert!(e.ends_with("ERR_PROJECT_NOT_FOUND: project 'beta' not found"), "got: {e}");
}

/// SPEC-0045/E2E-110: access outranks the directory problem, which outranks
/// the missing second file.
#[tokio::test]
async fn spec_0045_e2e_110_several_problems_fixed_order() {
    let f = world().await;
    let bob = err_text(diff(&f, BOB, ("alpha", "/notes"), ("beta", "/cfg/nothing.txt")).await);
    assert!(
        bob.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"),
        "got: {bob}"
    );
    let alice = err_text(diff(&f, ALICE, ("alpha", "/notes"), ("beta", "/cfg/nothing.txt")).await);
    assert!(alice.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory"), "got: {alice}");
}

/// SPEC-0045/E2E-111: climbing above the top stops at the top.
#[tokio::test]
async fn spec_0045_e2e_111_climbing_above_the_top() {
    let f = world().await;
    let e = err_text(
        diff(&f, ALICE, ("alpha", "/notes/../../etc/passwd"), ("alpha", "/notes/b.txt")).await,
    );
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/etc/passwd' not found"), "got: {e}");
}

/// SPEC-0045/E2E-212: the second path is a directory.
#[tokio::test]
async fn spec_0045_e2e_212_second_path_is_a_directory() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes")).await);
    assert!(e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory"), "got: {e}");
    assert!(!e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-217: first file in a deleted project (green on arrival).
#[tokio::test]
async fn spec_0045_e2e_217_first_file_in_a_deleted_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "x\n").await;
    f.admin_store.soft_delete_project("beta").await.unwrap();
    let e = err_text(diff(&f, ALICE, ("beta", "/cfg/app.txt"), ("alpha", "/notes/a.txt")).await);
    assert!(e.ends_with("ERR_PROJECT_NOT_FOUND: project 'beta' not found"), "got: {e}");
}

/// SPEC-0045/E2E-218: a deleted project brought back compares as before
/// (green on arrival).
#[tokio::test]
async fn spec_0045_e2e_218_deleted_project_brought_back() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    f.admin_store.soft_delete_project("beta").await.unwrap();
    f.state.admin.undelete_project("beta").await.unwrap();
    let v = ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/cfg/app.txt\n+++ beta:/cfg/app.txt\n@@ -1 +1 @@\n-port=80\n+port=8080\n"
    );
}

/// SPEC-0045/E2E-219: missing second file in another project.
#[tokio::test]
async fn spec_0045_e2e_219_missing_second_file_in_another_project() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/missing.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'beta:/cfg/missing.txt' not found"), "got: {e}");
}

/// SPEC-0045/E2E-222: climbing above the top, second file, other project.
#[tokio::test]
async fn spec_0045_e2e_222_climbing_above_the_top_second_file() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("beta", "/../secret.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'beta:/secret.txt' not found"), "got: {e}");
}

/// SPEC-0045/E2E-226: path normalization does not outrank access (green on
/// arrival).
#[tokio::test]
async fn spec_0045_e2e_226_normalization_does_not_outrank_access() {
    let f = world().await;
    let e = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/../x.txt")).await);
    assert!(e.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"), "got: {e}");
}

/// SPEC-0045/E2E-228: a NUL in a path is refused before any lookup (green on
/// arrival).
#[tokio::test]
async fn spec_0045_e2e_228_path_with_a_nul_character() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt\0")).await);
    assert!(e.ends_with("ERR_PATH_OUT_OF_BOUNDS: path contains a NUL byte"), "got: {e}");
}

/// SPEC-0045/E2E-229: an empty path means the root, which is a directory.
#[tokio::test]
async fn spec_0045_e2e_229_empty_path() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "")).await);
    assert!(e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/' is a directory"), "got: {e}");
}

async fn put_bytes(f: &Fixture, project: &str, path: &str, data: &[u8]) {
    let client = f.state.stores.client(project).await.unwrap();
    client.write_bytes_atomic(path, data).await.unwrap();
}

const OVER_LIMIT: &str = "ERR_INVALID_ARGUMENT: 'alpha:/logs/big.log' is 5242881 bytes, over the diff limit of 5242880 bytes";

/// SPEC-0045/E2E-104: a NUL byte makes the file not text.
#[tokio::test]
async fn spec_0045_e2e_104_a_file_with_a_nul_byte() {
    let f = world().await;
    put_bytes(&f, "alpha", "/img/logo.png", &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x00])
        .await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/img/logo.png")).await);
    assert!(
        e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/img/logo.png' is not a text file"),
        "got: {e}"
    );
    assert!(!e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-105: invalid UTF-8 without a NUL.
#[tokio::test]
async fn spec_0045_e2e_105_invalid_utf8() {
    let f = world().await;
    put_bytes(&f, "alpha", "/data/latin1.txt", &[0x63, 0x61, 0x66, 0xE9]).await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/data/latin1.txt"), ("alpha", "/notes/a.txt")).await);
    assert!(
        e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/data/latin1.txt' is not a text file"),
        "got: {e}"
    );
}

/// SPEC-0045/E2E-106: over the default limit, then fine after the operator
/// raises it and restarts (a new fixture models the restart).
#[tokio::test]
async fn spec_0045_e2e_106_a_file_over_the_limit() {
    let f = world().await;
    let big = vec![b'a'; 5_242_881];
    put_bytes(&f, "alpha", "/logs/big.log", &big).await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/logs/big.log")).await);
    assert!(e.ends_with(OVER_LIMIT), "got: {e}");

    let g = Fixture::with_config(|c| {
        c.safety.max_diff_file_bytes = crate::config::DiffLimit::new(6_291_456)
    })
    .await;
    g.seed_project("alpha", OWNER).await;
    g.state.admin.add_member("alpha", ALICE, OWNER).await.unwrap();
    put(&g, "alpha", "/notes/a.txt", "one\ntwo\nthree\n").await;
    put_bytes(&g, "alpha", "/logs/big.log", &big).await;
    let v = ok_json(diff(&g, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/logs/big.log")).await);
    assert_eq!(v["identical"], false);
}

/// SPEC-0045/E2E-204: exactly at the limit is accepted.
#[tokio::test]
async fn spec_0045_e2e_204_exactly_at_the_limit() {
    let f = world().await;
    put_bytes(&f, "alpha", "/logs/edge.log", &vec![b'a'; 5_242_880]).await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/logs/edge.log"), ("alpha", "/notes/a.txt")).await);
    assert_eq!(v["identical"], false);
}

/// SPEC-0045/E2E-205: one byte over the limit is refused.
#[tokio::test]
async fn spec_0045_e2e_205_one_byte_over_the_limit() {
    let f = world().await;
    put_bytes(&f, "alpha", "/logs/over.log", &vec![b'a'; 5_242_881]).await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/logs/over.log")).await);
    assert!(
        e.ends_with(
            "ERR_INVALID_ARGUMENT: 'alpha:/logs/over.log' is 5242881 bytes, over the diff limit of 5242880 bytes"
        ),
        "got: {e}"
    );
}

// ── US-0058: read only, and a comparison counts as access ───────────────────

/// Content and used bytes of one project file, the "nothing changed" snapshot.
async fn snapshot(f: &Fixture, project: &str, path: &str) -> (Vec<u8>, i64) {
    let client = f.state.stores.client(project).await.unwrap();
    let bytes = client.read_bytes(path).await.unwrap();
    (bytes, crate::quota::used_bytes(&client).await.unwrap())
}

async fn atime(f: &Fixture, project: &str, path: &str) -> f64 {
    f.state.stores.client(project).await.unwrap().stat(path).await.unwrap().atime
}

/// Poll up to 2 s (FR-NEW-012) for the atime of `path` to pass `before`.
async fn atime_after(f: &Fixture, project: &str, path: &str, before: f64) -> bool {
    for _ in 0..40 {
        if atime(f, project, path).await > before {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

/// The seeded atime is the write instant; waiting a little makes "newer"
/// distinguishable from it without touching test only meta setters.
async fn let_time_pass() {
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
}

fn edit_args(path: &str, old: &str, new: &str) -> EditArgs {
    EditArgs {
        mount_id: "alpha".into(),
        path: path.into(),
        old_string: old.into(),
        new_string: new.into(),
        replace_all: false,
        dry_run: false,
    }
}

async fn edit(
    f: &Fixture,
    path: &str,
    old: &str,
    new: &str,
) -> Result<CallToolResult, rmcp::ErrorData> {
    McpServer::new(f.state.clone(), ALICE.to_string())
        .fs_edit(Parameters(edit_args(path, old, new)))
        .await
}

async fn read_as_alice(f: &Fixture, path: &str) {
    let r = McpServer::new(f.state.clone(), ALICE.to_string())
        .fs_read(Parameters(ReadArgs {
            mount_id: "alpha".into(),
            path: path.into(),
            offset_lines: 0,
            limit_lines: 10,
            line_numbered: false,
        }))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "read failed: {}", text_of(&r));
}

/// SPEC-0045/E2E-005: comparing changes nothing. No accessor exposes the session
/// write allowance, so it is observed through behavior: with a 10 byte allowance,
/// three comparisons must leave all 10 bytes for a write that needs exactly 10.
#[tokio::test]
async fn spec_0045_e2e_005_comparing_changes_nothing() {
    let f = Fixture::with_config(|c| c.safety.write_quota_bytes = 10).await;
    f.seed_project("alpha", OWNER).await;
    f.state.admin.add_member("alpha", ALICE, OWNER).await.unwrap();
    put(&f, "alpha", "/notes/a.txt", "one\ntwo\nthree\n").await;
    put(&f, "alpha", "/notes/b.txt", "one\n2\nthree\n").await;
    let a = snapshot(&f, "alpha", "/notes/a.txt").await;
    let b = snapshot(&f, "alpha", "/notes/b.txt").await;
    for _ in 0..3 {
        ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    }
    assert_eq!(snapshot(&f, "alpha", "/notes/a.txt").await, a);
    assert_eq!(snapshot(&f, "alpha", "/notes/b.txt").await, b);

    let server = McpServer::new(f.state.clone(), ALICE.to_string());
    let w = server
        .fs_write(Parameters(crate::mcp::server::WriteArgs {
            mount_id: "alpha".into(),
            path: "/notes/q.txt".into(),
            content: "0123456789".into(),
            overwrite: false,
            create_parents: true,
        }))
        .await
        .unwrap();
    assert_ne!(w.is_error, Some(true), "the whole allowance must still be there: {}", text_of(&w));
    // Proves the 10 byte allowance is really enforced, so the success above means something.
    let over = server
        .fs_write(Parameters(crate::mcp::server::WriteArgs {
            mount_id: "alpha".into(),
            path: "/notes/q2.txt".into(),
            content: "x".into(),
            overwrite: false,
            create_parents: true,
        }))
        .await
        .unwrap();
    assert_eq!(over.is_error, Some(true), "the allowance is exhausted after 10 bytes");
}

/// SPEC-0045/E2E-006: last access updated on success, untouched on a refusal.
#[tokio::test]
async fn spec_0045_e2e_006_last_access_updated() {
    let f = world().await;
    let_time_pass().await;
    let (a0, b0) =
        (atime(&f, "alpha", "/notes/a.txt").await, atime(&f, "alpha", "/notes/b.txt").await);
    ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    assert!(atime_after(&f, "alpha", "/notes/a.txt", a0).await, "a.txt atime not bumped");
    assert!(atime_after(&f, "alpha", "/notes/b.txt", b0).await, "b.txt atime not bumped");

    let a1 = atime(&f, "alpha", "/notes/a.txt").await;
    err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/missing.txt")).await);
    // A fire and forget bump would land within this window if it were wrongly issued.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(atime(&f, "alpha", "/notes/a.txt").await, a1, "a refusal must not touch");
}

/// SPEC-0045/E2E-007: comparing is not reading.
#[tokio::test]
async fn spec_0045_e2e_007_comparing_is_not_reading() {
    let f = world().await;
    ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    let e = err_text(edit(&f, "/notes/a.txt", "two", "2").await);
    assert!(
        e.ends_with("ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session"),
        "got: {e}"
    );
    read_as_alice(&f, "/notes/a.txt").await;
    ok_json(edit(&f, "/notes/a.txt", "two", "2").await);
}

/// SPEC-0045/E2E-213: a cross project comparison, and a refused one, change nothing.
#[tokio::test]
async fn spec_0045_e2e_213_cross_project_comparison_changes_nothing() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    let a = snapshot(&f, "alpha", "/cfg/app.txt").await;
    let b = snapshot(&f, "beta", "/cfg/app.txt").await;
    ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    err_text(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/missing.txt")).await);
    assert_eq!(snapshot(&f, "alpha", "/cfg/app.txt").await, a);
    assert_eq!(snapshot(&f, "beta", "/cfg/app.txt").await, b);
}

/// SPEC-0045/E2E-214: the second file is not read either.
#[tokio::test]
async fn spec_0045_e2e_214_edit_of_the_second_file_after_a_comparison() {
    let f = world().await;
    ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    let e = err_text(edit(&f, "/notes/b.txt", "2", "two").await);
    assert!(
        e.ends_with("ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/b.txt' requires reading it first in this session"),
        "got: {e}"
    );
}

/// SPEC-0045/E2E-215: last access is bumped in each project.
#[tokio::test]
async fn spec_0045_e2e_215_last_access_in_each_project() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    let_time_pass().await;
    let (a0, b0) =
        (atime(&f, "alpha", "/cfg/app.txt").await, atime(&f, "beta", "/cfg/app.txt").await);
    ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert!(atime_after(&f, "alpha", "/cfg/app.txt", a0).await, "alpha atime not bumped");
    assert!(atime_after(&f, "beta", "/cfg/app.txt", b0).await, "beta atime not bumped");
}

/// SPEC-0045/E2E-216: identical files still count as accessed, still not read.
#[tokio::test]
async fn spec_0045_e2e_216_identical_files_still_count_as_accessed() {
    let f = world().await;
    put(&f, "alpha", "/notes/copy.txt", "one\ntwo\nthree\n").await;
    let_time_pass().await;
    let (a0, c0) =
        (atime(&f, "alpha", "/notes/a.txt").await, atime(&f, "alpha", "/notes/copy.txt").await);
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/copy.txt")).await);
    assert_eq!(v["identical"], true);
    assert!(atime_after(&f, "alpha", "/notes/a.txt", a0).await, "a.txt atime not bumped");
    assert!(atime_after(&f, "alpha", "/notes/copy.txt", c0).await, "copy.txt atime not bumped");
    let e = err_text(edit(&f, "/notes/a.txt", "two", "2").await);
    assert!(
        e.ends_with("ERR_EDIT_WITHOUT_PRIOR_READ: edit '/notes/a.txt' requires reading it first in this session"),
        "got: {e}"
    );
}

// ── US-0059: the same comparison on the REST data plane ─────────────────────

/// The real data plane router over the fixture's state, plus a bearer token
/// for `person` (a fresh keypair per call, as in the quota REST tests).
fn rest(f: &Fixture, person: &str) -> (axum::Router, String) {
    let keys = tempfile::tempdir().unwrap();
    let (key, public) = crate::keys::write_keypair(keys.path()).unwrap();
    let mut config = (*f.state.config).clone();
    config.auth.jwt.public_key_path = public.display().to_string();
    let s = &f.state;
    let state = crate::state::AppState {
        config: std::sync::Arc::new(config.clone()),
        admin: s.admin.clone(),
        stores: s.stores.clone(),
        safety: s.safety.clone(),
        identity: std::sync::Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
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
    (crate::api::dataplane::router(std::sync::Arc::new(state)), token)
}

/// `GET /api/fs/diff?<query>` as `person`; the status and the JSON body.
async fn rest_diff(f: &Fixture, person: &str, query: &str) -> (u16, Value) {
    use tower::ServiceExt as _;
    let (app, token) = rest(f, person);
    let req = axum::http::Request::builder()
        .uri(format!("/api/fs/diff?{query}"))
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

/// SPEC-0045/E2E-004: a program gets the tool's answer, field by field.
#[tokio::test]
async fn spec_0045_e2e_004_program_client_gets_the_same_answer() {
    let f = world().await;
    let tool = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    let (status, body) = rest_diff(
        &f,
        ALICE,
        "mount_id_a=alpha&path_a=/notes/a.txt&mount_id_b=alpha&path_b=/notes/b.txt",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["diff"], tool["diff"]);
    assert_eq!(body["identical"], tool["identical"]);
    assert_eq!(body, tool);
}

/// SPEC-0045/E2E-112: a program gets the tool's refusal, with its status.
#[tokio::test]
async fn spec_0045_e2e_112_program_client_gets_the_same_refusal() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "secret=1\n").await;
    let tool = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/cfg/app.txt")).await);
    let (status, body) = rest_diff(
        &f,
        BOB,
        "mount_id_a=alpha&path_a=/notes/a.txt&mount_id_b=beta&path_b=/cfg/app.txt",
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"], "ERR_FORBIDDEN");
    let detail = body["detail"].as_str().expect("a detail string");
    assert_eq!(detail, "ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'");
    assert!(tool.ends_with(detail), "tool: {tool}, rest: {detail}");
    assert!(!detail.contains("secret"));
}

/// SPEC-0045/E2E-220: identical files through the web interface.
#[tokio::test]
async fn spec_0045_e2e_220_identical_files_through_the_web_interface() {
    let f = world().await;
    put(&f, "alpha", "/notes/copy.txt", "one\ntwo\nthree\n").await;
    let (status, body) = rest_diff(
        &f,
        ALICE,
        "mount_id_a=alpha&path_a=/notes/a.txt&mount_id_b=alpha&path_b=/notes/copy.txt",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"identical": true, "diff": ""}));
}

/// Asserts a REST refusal is a 400 `ERR_INVALID_ARGUMENT` naming `name`.
fn assert_required(status: u16, body: &Value, name: &str) {
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "ERR_INVALID_ARGUMENT");
    assert_eq!(body["detail"], format!("ERR_INVALID_ARGUMENT: '{name}' is required"));
}

/// SPEC-0045/E2E-223: second project missing.
#[tokio::test]
async fn spec_0045_e2e_223_web_interface_second_project_missing() {
    let f = world().await;
    let (status, body) =
        rest_diff(&f, ALICE, "mount_id_a=alpha&path_a=/notes/a.txt&path_b=/notes/b.txt").await;
    assert_required(status, &body, "mount_id_b");
}

/// SPEC-0045/E2E-224: several values missing, the first in the fixed order wins.
#[tokio::test]
async fn spec_0045_e2e_224_web_interface_several_values_missing() {
    let f = world().await;
    let (status, body) = rest_diff(&f, ALICE, "mount_id_a=alpha&path_a=").await;
    assert_required(status, &body, "path_a");
}

/// SPEC-0045/E2E-227: a value of spaces counts as empty.
#[tokio::test]
async fn spec_0045_e2e_227_value_of_spaces_on_the_web_interface() {
    let f = world().await;
    let (status, body) = rest_diff(
        &f,
        ALICE,
        "mount_id_a=alpha&path_a=/notes/a.txt&mount_id_b=alpha&path_b=%20%20%20",
    )
    .await;
    assert_required(status, &body, "path_b");
}

// ── US-0060: the 10 second bound ────────────────────────────────────────────

/// 65536 lines of 80 bytes (79 characters and a line feed) is exactly the
/// 5 MiB default limit; every 65th line (1000 of them) carries `marker`.
fn big_text(marker: char) -> String {
    let mut text = String::with_capacity(5_242_880);
    for i in 0..65_536usize {
        let c = if i % 65 == 0 && i / 65 < 1000 { marker } else { 'x' };
        let head = format!("{i:06} ");
        text.push_str(&head);
        text.extend(std::iter::repeat_n(c, 79 - head.len()));
        text.push('\n');
    }
    text
}

/// SPEC-0045/E2E-230: two 5 MiB files differing on 1000 spread lines are
/// compared within 10 seconds.
#[tokio::test]
async fn spec_0045_e2e_230_large_comparison_within_the_time_bound() {
    let f = world().await;
    let (old, new) = (big_text('a'), big_text('b'));
    assert_eq!((old.len(), new.len()), (5_242_880, 5_242_880));
    put(&f, "alpha", "/p1.txt", &old).await;
    put(&f, "alpha", "/p2.txt", &new).await;

    let started = std::time::Instant::now();
    let v = ok_json(diff(&f, ALICE, ("alpha", "/p1.txt"), ("alpha", "/p2.txt")).await);
    let elapsed = started.elapsed();
    eprintln!("E2E-230 wall time: {elapsed:?}");

    assert_eq!(v["identical"], false);
    let body = v["diff"].as_str().unwrap();
    let (mut removed, mut added) = (0, 0);
    for line in body.lines().skip(2) {
        match line.as_bytes().first() {
            Some(b'-') => removed += 1,
            Some(b'+') => added += 1,
            _ => {}
        }
    }
    assert_eq!((removed, added), (1000, 1000));
    assert!(elapsed < std::time::Duration::from_secs(10), "took {elapsed:?}");
}

// ── SPEC-0046 US-0061: context_lines ────────────────────────────────────────

const SIZE_REFUSAL: &str =
    "ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000";

fn numbered(n: usize, replace: &[usize]) -> String {
    (1..=n)
        .map(|i| if replace.contains(&i) { format!("L{i}\n") } else { format!("l{i}\n") })
        .collect()
}

/// P1 to P7 of SPEC-0046, all in alpha.
async fn world_46() -> Fixture {
    let f = world().await;
    f.state.admin.add_member("beta", ALICE, OWNER).await.unwrap();
    put(&f, "alpha", "/a.txt", &numbered(10, &[])).await;
    put(&f, "alpha", "/b.txt", &numbered(10, &[6])).await;
    put(&f, "alpha", "/same.txt", &numbered(10, &[])).await;
    put(&f, "beta", "/b.txt", &numbered(10, &[6])).await;
    put(&f, "alpha", "/c.txt", &numbered(14, &[])).await;
    put(&f, "alpha", "/d.txt", &numbered(14, &[3, 12])).await;
    put(&f, "alpha", "/e.txt", &numbered(10, &[])).await;
    put(&f, "alpha", "/f.txt", &numbered(10, &[1])).await;
    put(&f, "alpha", "/g.txt", &numbered(15, &[])).await;
    put(&f, "alpha", "/h.txt", &numbered(15, &[3, 10])).await;
    put(&f, "alpha", "/i.txt", &numbered(16, &[])).await;
    put(&f, "alpha", "/j.txt", &numbered(16, &[3, 11])).await;
    put(&f, "alpha", "/k.txt", &numbered(5, &[])).await;
    put(&f, "alpha", "/m.txt", &numbered(5, &[2, 3])).await;
    f
}

async fn diff_text(f: &Fixture, a: (&str, &str), b: (&str, &str), size: Option<Value>) -> Value {
    ok_json(diff_with(f, ALICE, a, b, size).await)
}

async fn assert_diff(a: &str, b: &str, size: Option<Value>, expected: &str) {
    let f = world_46().await;
    let v = diff_text(&f, ("alpha", a), ("alpha", b), size).await;
    assert_eq!(v["diff"], expected);
    assert_eq!(v["identical"], false);
}

async fn assert_refused(size: Value) {
    let f = world_46().await;
    let e =
        err_text(diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(size)).await);
    assert!(e.ends_with(SIZE_REFUSAL), "got: {e}");
    assert!(!e.contains("@@"), "no diff may be returned: {e}");
}

const ONE: &str = "--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -5,3 +5,3 @@\n l5\n-l6\n+L6\n l7\n";

#[tokio::test]
async fn spec_0046_e2e_001_size_1() {
    assert_diff("/a.txt", "/b.txt", Some(json!(1)), ONE).await;
}

#[tokio::test]
async fn spec_0046_e2e_002_no_size() {
    assert_diff(
        "/a.txt",
        "/b.txt",
        None,
        "--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -3,7 +3,7 @@\n l3\n l4\n l5\n-l6\n+L6\n l7\n l8\n l9\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_003_size_0() {
    assert_diff(
        "/a.txt",
        "/b.txt",
        Some(json!(0)),
        "--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -6 +6 @@\n-l6\n+L6\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_004_two_projects_size_1() {
    let f = world_46().await;
    let v = diff_text(&f, ("alpha", "/a.txt"), ("beta", "/b.txt"), Some(json!(1))).await;
    assert_eq!(v["diff"], ONE.replace("+++ alpha:/b.txt", "+++ beta:/b.txt"));
}

#[tokio::test]
async fn spec_0046_e2e_006_largest_accepted_size() {
    assert_diff(
        "/a.txt",
        "/b.txt",
        Some(json!(10_000_000)),
        "--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -1,10 +1,10 @@\n l1\n l2\n l3\n l4\n l5\n-l6\n+L6\n l7\n l8\n l9\n l10\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_007_changes_8_apart_size_3() {
    assert_diff(
        "/c.txt",
        "/d.txt",
        Some(json!(3)),
        "--- alpha:/c.txt\n+++ alpha:/d.txt\n@@ -1,6 +1,6 @@\n l1\n l2\n-l3\n+L3\n l4\n l5\n l6\n@@ -9,6 +9,6 @@\n l9\n l10\n l11\n-l12\n+L12\n l13\n l14\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_008_same_changes_size_4() {
    assert_diff(
        "/c.txt",
        "/d.txt",
        Some(json!(4)),
        "--- alpha:/c.txt\n+++ alpha:/d.txt\n@@ -1,14 +1,14 @@\n l1\n l2\n-l3\n+L3\n l4\n l5\n l6\n l7\n l8\n l9\n l10\n l11\n-l12\n+L12\n l13\n l14\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_009_change_on_first_line() {
    assert_diff(
        "/e.txt",
        "/f.txt",
        Some(json!(3)),
        "--- alpha:/e.txt\n+++ alpha:/f.txt\n@@ -1,4 +1,4 @@\n-l1\n+L1\n l2\n l3\n l4\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_010_identical_size_0() {
    let f = world_46().await;
    let v = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/same.txt"), Some(json!(0))).await;
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

#[tokio::test]
async fn spec_0046_e2e_011_negative_size() {
    assert_refused(json!(-1)).await;
    // The same call without a size still returns the 3 line diff.
    let f = world_46().await;
    let v = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/b.txt"), None).await;
    assert!(v["diff"].as_str().unwrap().contains("@@ -3,7 +3,7 @@"));
}

#[tokio::test]
async fn spec_0046_e2e_012_not_a_number() {
    assert_refused(json!("abc")).await;
}

#[tokio::test]
async fn spec_0046_e2e_013_decimal() {
    assert_refused(json!(2.5)).await;
}

#[tokio::test]
async fn spec_0046_e2e_014_just_above_the_maximum() {
    assert_refused(json!(10_000_001)).await;
}

#[tokio::test]
async fn spec_0046_e2e_016_invalid_size_and_missing_file() {
    let f = world_46().await;
    let e = err_text(
        diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/missing.txt"), Some(json!(-1))).await,
    );
    assert!(e.ends_with(SIZE_REFUSAL), "got: {e}");
}

#[tokio::test]
async fn spec_0046_e2e_017_invalid_size_by_a_non_member() {
    let f = world_46().await;
    let mallory = "mallory@example.com";
    let e = err_text(
        diff_with(&f, mallory, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(json!("abc"))).await,
    );
    assert!(e.ends_with(SIZE_REFUSAL), "got: {e}");
    let alice = err_text(
        diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(json!("abc"))).await,
    );
    assert_eq!(e, alice);
}

#[tokio::test]
async fn spec_0046_e2e_019_refusal_touches_nothing() {
    let f = world_46().await;
    let_time_pass().await;
    let (a0, b0) = (atime(&f, "alpha", "/a.txt").await, atime(&f, "alpha", "/b.txt").await);
    let before = (snapshot(&f, "alpha", "/a.txt").await, snapshot(&f, "alpha", "/b.txt").await);
    let e = err_text(
        diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(json!(-1))).await,
    );
    assert!(e.ends_with(SIZE_REFUSAL), "got: {e}");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(atime(&f, "alpha", "/a.txt").await, a0);
    assert_eq!(atime(&f, "alpha", "/b.txt").await, b0);
    let after = (snapshot(&f, "alpha", "/a.txt").await, snapshot(&f, "alpha", "/b.txt").await);
    assert_eq!(before, after);
    let v = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(json!(1))).await;
    assert_eq!(v["diff"], ONE);
    assert!(atime_after(&f, "alpha", "/a.txt", a0).await, "a later success counts as access");
}

#[tokio::test]
async fn spec_0046_e2e_021_identical_files_invalid_size() {
    let f = world_46().await;
    let e = err_text(
        diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/same.txt"), Some(json!(-1))).await,
    );
    assert!(e.ends_with(SIZE_REFUSAL), "got: {e}");
}

#[tokio::test]
async fn spec_0046_e2e_022_identical_files_no_size() {
    let f = world_46().await;
    let v = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/same.txt"), None).await;
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

#[tokio::test]
async fn spec_0046_e2e_026_leading_plus_sign() {
    assert_refused(json!("+3")).await;
}

#[tokio::test]
async fn spec_0046_e2e_027_leading_zeros() {
    assert_diff("/a.txt", "/b.txt", Some(json!("001")), ONE).await;
}

#[tokio::test]
async fn spec_0046_e2e_028_null_size() {
    let f = world_46().await;
    let none = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/b.txt"), None).await;
    let null = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(Value::Null)).await;
    assert_eq!(none, null);
    assert!(null["diff"].as_str().unwrap().contains("@@ -3,7 +3,7 @@"));
}

#[tokio::test]
async fn spec_0046_e2e_029_number_3_point_0() {
    let raw: Value = serde_json::from_str("3.0").unwrap();
    assert_refused(raw).await;
}

#[tokio::test]
async fn spec_0046_e2e_030_number_1e3() {
    let raw: Value = serde_json::from_str("1e3").unwrap();
    assert_refused(raw).await;
}

#[tokio::test]
async fn spec_0046_e2e_032_leading_space() {
    assert_refused(json!(" 3")).await;
}

#[tokio::test]
async fn spec_0046_e2e_033_trailing_space() {
    assert_refused(json!("3 ")).await;
}

#[tokio::test]
async fn spec_0046_e2e_034_hex_text() {
    assert_refused(json!("0x3")).await;
}

#[tokio::test]
async fn spec_0046_e2e_035_boolean() {
    assert_refused(json!(true)).await;
}

#[tokio::test]
async fn spec_0046_e2e_036_list() {
    assert_refused(json!([1])).await;
}

#[tokio::test]
async fn spec_0046_e2e_037_number_and_text_agree() {
    assert_diff("/a.txt", "/b.txt", Some(json!(1)), ONE).await;
    assert_diff("/a.txt", "/b.txt", Some(json!("1")), ONE).await;
}

#[tokio::test]
async fn spec_0046_e2e_040_separate_changes_size_0() {
    assert_diff(
        "/c.txt",
        "/d.txt",
        Some(json!(0)),
        "--- alpha:/c.txt\n+++ alpha:/d.txt\n@@ -3 +3 @@\n-l3\n+L3\n@@ -12 +12 @@\n-l12\n+L12\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_041_separate_changes_size_1() {
    assert_diff(
        "/c.txt",
        "/d.txt",
        Some(json!(1)),
        "--- alpha:/c.txt\n+++ alpha:/d.txt\n@@ -2,3 +2,3 @@\n l2\n-l3\n+L3\n l4\n@@ -11,3 +11,3 @@\n l11\n-l12\n+L12\n l13\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_043_six_unchanged_lines_size_3() {
    assert_diff(
        "/g.txt",
        "/h.txt",
        Some(json!(3)),
        "--- alpha:/g.txt\n+++ alpha:/h.txt\n@@ -1,13 +1,13 @@\n l1\n l2\n-l3\n+L3\n l4\n l5\n l6\n l7\n l8\n l9\n-l10\n+L10\n l11\n l12\n l13\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_044_seven_unchanged_lines_size_3() {
    assert_diff(
        "/i.txt",
        "/j.txt",
        Some(json!(3)),
        "--- alpha:/i.txt\n+++ alpha:/j.txt\n@@ -1,6 +1,6 @@\n l1\n l2\n-l3\n+L3\n l4\n l5\n l6\n@@ -8,7 +8,7 @@\n l8\n l9\n l10\n-l11\n+L11\n l12\n l13\n l14\n",
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_045_identical_size_3() {
    let f = world_46().await;
    let v = diff_text(&f, ("alpha", "/a.txt"), ("alpha", "/same.txt"), Some(json!(3))).await;
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

#[tokio::test]
async fn spec_0046_e2e_046_adjacent_changes_size_0() {
    assert_diff(
        "/k.txt",
        "/m.txt",
        Some(json!(0)),
        "--- alpha:/k.txt\n+++ alpha:/m.txt\n@@ -2,2 +2,2 @@\n-l2\n-l3\n+L2\n+L3\n",
    )
    .await;
}

// ── SPEC-0046 US-0062: context_lines on the web interface ───────────────────

const AB: &str = "mount_id_a=alpha&path_a=/a.txt&mount_id_b=alpha&path_b=/b.txt";
const SIZE_DETAIL: &str =
    "ERR_INVALID_ARGUMENT: 'context_lines' must be a whole number from 0 to 10000000";

/// A web refusal: 400, the stable code and the pinned message, no diff.
async fn assert_web_refused(f: &Fixture, query: &str) {
    let (status, body) = rest_diff(f, ALICE, query).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "ERR_INVALID_ARGUMENT");
    assert_eq!(body["detail"], SIZE_DETAIL);
    assert!(body.get("diff").is_none(), "no diff may be returned: {body}");
}

/// The web answer for `size` equals the tool answer for the same size.
async fn assert_web_matches_tool(size: &str, tool_size: Value, expected: Option<&str>) {
    let f = world_46().await;
    let tool = ok_json(
        diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(tool_size)).await,
    );
    let (status, body) = rest_diff(&f, ALICE, &format!("{AB}&context_lines={size}")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, tool);
    if let Some(expected) = expected {
        assert_eq!(body["diff"], expected);
    }
}

#[tokio::test]
async fn spec_0046_e2e_005_web_size_1() {
    assert_web_matches_tool("1", json!(1), Some(ONE)).await;
}

#[tokio::test]
async fn spec_0046_e2e_015_web_empty_size() {
    let f = world_46().await;
    assert_web_refused(&f, &format!("{AB}&context_lines=")).await;
}

#[tokio::test]
async fn spec_0046_e2e_018_web_invalid_size() {
    let f = world_46().await;
    assert_web_refused(&f, &format!("{AB}&context_lines=abc")).await;
    let tool = err_text(
        diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(json!("abc"))).await,
    );
    assert!(tool.ends_with(SIZE_DETAIL), "{tool}");
}

#[tokio::test]
async fn spec_0046_e2e_023_web_missing_value_wins_over_invalid_size() {
    let f = world_46().await;
    let (status, body) =
        rest_diff(&f, ALICE, "mount_id_a=alpha&path_a=/a.txt&mount_id_b=alpha&context_lines=abc")
            .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["detail"], "ERR_INVALID_ARGUMENT: 'path_b' is required");
}

#[tokio::test]
async fn spec_0046_e2e_038_web_first_occurrence_counts() {
    let f = world_46().await;
    assert_web_refused(&f, &format!("{AB}&context_lines=&context_lines=2")).await;
}

#[tokio::test]
async fn spec_0046_e2e_039_web_one_space_size() {
    let f = world_46().await;
    assert_web_refused(&f, &format!("{AB}&context_lines=%20")).await;
}

#[tokio::test]
async fn spec_0046_e2e_042_negative_zero() {
    let f = world_46().await;
    // Alice gives the number -0 unquoted; the client gives the text -0.
    let raw: Value = serde_json::from_str("-0").unwrap();
    let e =
        err_text(diff_with(&f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(raw)).await);
    assert!(e.ends_with(SIZE_REFUSAL), "got: {e}");
    assert_web_refused(&f, &format!("{AB}&context_lines=-0")).await;
}

#[tokio::test]
async fn spec_0046_e2e_047_web_size_0() {
    assert_web_matches_tool(
        "0",
        json!(0),
        Some("--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -6 +6 @@\n-l6\n+L6\n"),
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_048_web_size_10000000() {
    assert_web_matches_tool(
        "10000000",
        json!(10_000_000),
        Some(
            "--- alpha:/a.txt\n+++ alpha:/b.txt\n@@ -1,10 +1,10 @@\n l1\n l2\n l3\n l4\n l5\n-l6\n+L6\n l7\n l8\n l9\n l10\n",
        ),
    )
    .await;
}

#[tokio::test]
async fn spec_0046_e2e_049_web_leading_zeros() {
    assert_web_matches_tool("001", json!("001"), Some(ONE)).await;
}

// ── SPEC-0046 US-0063: other diffs unchanged, scale ─────────────────────────

/// The comparison with size 0 that precedes each write family check: it must
/// leave the 3 line default of write, edit and multi edit alone (DEC-007).
async fn compare_with_size_0(f: &Fixture) {
    let v = ok_json(
        diff_with(f, ALICE, ("alpha", "/a.txt"), ("alpha", "/b.txt"), Some(json!(0))).await,
    );
    assert!(v["diff"].as_str().unwrap().contains("@@ -6 +6 @@"));
}

const THREE_AROUND_LINE_6: &str = "@@ -3,7 +3,7 @@";

#[tokio::test]
async fn spec_0046_e2e_020_write_after_comparison() {
    let f = world_46().await;
    compare_with_size_0(&f).await;
    read_as_alice(&f, "/a.txt").await;
    let v = ok_json(
        McpServer::new(f.state.clone(), ALICE.to_string())
            .fs_write(Parameters(WriteArgs {
                mount_id: "alpha".into(),
                path: "/a.txt".into(),
                content: numbered(10, &[]).replace("l6\n", "X6\n"),
                overwrite: true,
                create_parents: true,
            }))
            .await,
    );
    let d = v["diff"].as_str().unwrap();
    assert!(d.contains(THREE_AROUND_LINE_6), "got: {d}");
    assert!(d.contains(" l3\n l4\n l5\n-l6\n+X6\n l7\n l8\n l9\n"), "got: {d}");
}

#[tokio::test]
async fn spec_0046_e2e_024_edit_after_comparison() {
    let f = world_46().await;
    compare_with_size_0(&f).await;
    read_as_alice(&f, "/a.txt").await;
    let v = ok_json(edit(&f, "/a.txt", "l6", "X6").await);
    let d = v["diff"].as_str().unwrap();
    assert!(d.contains(THREE_AROUND_LINE_6), "got: {d}");
    assert!(d.contains(" l3\n l4\n l5\n-l6\n+X6\n l7\n l8\n l9\n"), "got: {d}");
}

#[tokio::test]
async fn spec_0046_e2e_025_multi_edit_after_comparison() {
    let f = world_46().await;
    compare_with_size_0(&f).await;
    read_as_alice(&f, "/a.txt").await;
    let v = ok_json(
        McpServer::new(f.state.clone(), ALICE.to_string())
            .fs_multi_edit(Parameters(MultiEditArgs {
                mount_id: "alpha".into(),
                path: "/a.txt".into(),
                edits: json!([{"old_string": "l6", "new_string": "X6"}]),
                dry_run: false,
            }))
            .await,
    );
    let d = v["diff"].as_str().unwrap();
    assert!(d.contains(THREE_AROUND_LINE_6), "got: {d}");
    assert!(d.contains(" l3\n l4\n l5\n-l6\n+X6\n l7\n l8\n l9\n"), "got: {d}");
}

/// Two files exactly at the 5242880 byte limit, 2621440 lines each, differing
/// on line 1 only: one block spanning every line.
#[tokio::test]
async fn spec_0046_e2e_031_two_files_at_the_size_limit() {
    let f = world_46().await;
    let first = "x\n".repeat(2_621_440).into_bytes();
    let mut second = first.clone();
    second[0] = b'y';
    assert_eq!(first.len(), 5_242_880);
    put_bytes(&f, "alpha", "/big1.txt", &first).await;
    put_bytes(&f, "alpha", "/big2.txt", &second).await;
    let v = ok_json(
        diff_with(
            &f,
            ALICE,
            ("alpha", "/big1.txt"),
            ("alpha", "/big2.txt"),
            Some(json!(10_000_000)),
        )
        .await,
    );
    let d = v["diff"].as_str().unwrap();
    assert_eq!(d.matches("@@ -1,2621440 +1,2621440 @@").count(), 1);
    assert_eq!(d.matches("\n@@ ").count(), 1, "one block only");
}
