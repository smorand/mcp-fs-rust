//! SPEC-0045 US-0055 acceptance tests: `fs.diff_files`, driven through the real
//! `McpServer::fs_diff_files` handler over real storage (SQLite metadata, local
//! blobs). Nothing is mocked.

use crate::mcp::server::{DiffFilesArgs, McpServer};
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
    let server = McpServer::new(f.state.clone(), person.to_string());
    server
        .fs_diff_files(Parameters(DiffFilesArgs {
            mount_id_a: a.0.to_string(),
            path_a: a.1.to_string(),
            mount_id_b: b.0.to_string(),
            path_b: b.1.to_string(),
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
