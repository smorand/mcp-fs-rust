//! SPEC-0089 tests for the document converter's text answer (US-0218): the
//! real `CliDocService` runs over stand in shell scripts, through
//! `core::fs_ops::documentize`. Each exploit test carries its literal payload
//! (1 073 741 824 letters `a` on the converter's standard output) and checks the
//! second channels: the `CONVERTER_HELD` hook, the stand in's pid probed with
//! `kill -0`, its working folder, the listing of `/docs`, the artifact lists,
//! the audit trail and a following 52 428 800 byte write. Every flood test
//! takes `heavy()` first: before the fix the answer is read whole.

use crate::docs::service::{CONVERTER_HELD, CliDocService, ConverterHeld};
use crate::errors::{Result, code};
use crate::tools::admin::test_support::Fixture;
use crate::tools::archive_testkit::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The converter limit of every test here, the figure the refusals name.
const CAP: u64 = 1_048_576;
/// The cap plus one 65 536 byte chunk: the most an answer may be held at.
const HELD_MAX: u64 = 1_114_112;
/// The refusal of an answer over [`CAP`].
const OVER: &str =
    "document service output is over the doc_service.max_input_bytes limit of 1048576";
/// The flood of SEC-013 and E2E-046: 1 073 741 824 letters `a`.
const FLOOD: &str = "exec perl -e 'my $s = \"a\" x 65536; print $s for 1..16384'\n";

/// An executable `sh` script in `home` that first writes `started`, its pid and
/// its working folder to `home/marker` (outside the sandbox), then runs `body`.
/// A flood ends in `exec perl` so the flooding process is the one killed.
fn stand_in(home: &Path, body: &str) -> PathBuf {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    let path = home.join("conv.sh");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(
        b"#!/bin/sh\nm=\"$(dirname \"$0\")/marker\"\nprintf 'started\\n%s\\n%s\\n' \"$$\" \"$(pwd)\" >> \"$m\"\n",
    )
    .unwrap();
    f.write_all(body.as_bytes()).unwrap();
    drop(f);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn cli(command: &Path, cap: u64, timeout_secs: u64) -> CliDocService {
    CliDocService::new(
        crate::config::DocServiceCliConfig {
            command: vec![command.display().to_string(), crate::config::DOC_PLACEHOLDER.into()],
            timeout_secs,
        },
        Vec::new(),
        cap,
    )
}

/// `(pid, working folder)` of every start the stand in recorded (the marker is
/// appended to, so two calls sharing one script leave two records).
fn markers(home: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(home.join("marker")) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    lines
        .chunks(3)
        .map(|c| {
            assert_eq!(c[0], "started");
            (c[1].to_string(), PathBuf::from(c[2]))
        })
        .collect()
}

fn marker(home: &Path) -> Option<(String, PathBuf)> {
    markers(home).into_iter().next()
}

/// `kill -0 <pid>` succeeds only for a live (or zombie) process.
fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// The stand in started, its process is gone and its working folder is absent.
fn stopped_and_cleaned(home: &Path) -> usize {
    let all = markers(home);
    assert!(!all.is_empty(), "the stand in started");
    for (pid, cwd) in &all {
        assert!(!alive(pid), "the stand in converter {pid} is still running");
        assert!(!cwd.exists(), "the working folder {} still exists", cwd.display());
    }
    all.len()
}

/// Project `atlas`, member `mia@example.com`, a session write allowance of
/// 1 000 000 000 and `/docs/deck.pptx` holding `PK`.
async fn converter_fixture() -> Fixture {
    let f = office_fixture_allowance(1_000_000_000).await;
    seed(&f, "/docs/deck.pptx", b"PK").await;
    f
}

/// Authorizes then converts, as both doors do.
async fn ask(
    f: &Fixture,
    svc: &CliDocService,
    person: &str,
    path: &str,
    overwrite: bool,
) -> Result<Value> {
    f.state.authorize(MOUNT, person).await.unwrap();
    let c = f.state.stores.client(MOUNT).await.unwrap();
    crate::core::fs_ops::documentize(&c, &f.state.safety, Some(svc), person, MOUNT, path, overwrite)
        .await
}

fn held_reset() {
    CONVERTER_HELD.with(|v| v.borrow_mut().clear());
}

fn holds() -> Vec<ConverterHeld> {
    CONVERTER_HELD.with(|v| v.borrow().clone())
}

/// The answer the single call held, at most [`HELD_MAX`].
fn answer_held() -> u64 {
    let all = holds();
    assert_eq!(all.len(), 1, "one record per call: {all:?}");
    all[0].answer
}

fn refused(v: &Result<Value>, message: &str) {
    let Err(e) = v else { panic!("the conversion must be refused: {v:?}") };
    assert_eq!(e.code, code::INTERNAL_ERROR, "{e:?}");
    assert_eq!(e.message, message);
}

/// No companion, artifact or audit line of `doc` remains and the session
/// allowance is untouched (`nothing_left` writes 52 428 800 bytes after).
async fn clean(f: &Fixture, doc: &str) {
    nothing_left(f, doc, None).await;
    let log = f.state.safety.audit(OWNER, MOUNT);
    assert!(!log.iter().any(|e| e.op == "doc_service"), "{log:?}");
}

#[tokio::test]
async fn e2e_0089_sec_013_text_answer_flood() {
    let _heavy = heavy().await;
    let f = converter_fixture().await;
    seed(&f, "/docs/bomb.docx", b"PK").await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&stand_in(home.path(), FLOOD), CAP, 120);
    held_reset();

    let v = ask(&f, &svc, OWNER, "/docs/bomb.docx", false).await;

    let held = answer_held();
    assert!(held <= HELD_MAX, "the answer held {held} bytes, more than {HELD_MAX}");
    refused(&v, OVER);
    stopped_and_cleaned(home.path());
    clean(&f, "/docs/bomb.docx").await;
}

#[tokio::test]
async fn e2e_0089_003_text_mode_conversion_within_the_limit() {
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(
        &stand_in(home.path(), "printf '# Deck'\nexec perl -e 'print \"d\" x 1994'\n"),
        CAP,
        120,
    );

    let v = ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await.unwrap();

    assert_eq!(v["md_path"], json!("/docs/deck.md"));
    assert_eq!(v["bytes_written"], json!(2000));
    assert_eq!(v["overwritten"], json!(false));
    let md = read(&f, "/docs/deck.md").await.unwrap();
    assert_eq!(md, [b"# Deck".as_slice(), &vec![b'd'; 1994]].concat());
    let log = f.state.safety.audit(OWNER, MOUNT);
    let lines = log.iter().filter(|e| e.op == "doc_service" && e.path == "/docs/deck.md");
    assert_eq!(lines.count(), 1, "{log:?}");
}

/// A stand in answering `#` then `n` letters `d`.
fn hashes(home: &Path, n: u64) -> PathBuf {
    stand_in(home, &format!("printf '#'\nexec perl -e 'print \"d\" x {n}'\n"))
}

#[tokio::test]
async fn e2e_0089_033_text_answer_of_exactly_the_converter() {
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&hashes(home.path(), CAP - 1), CAP, 120);

    ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await.unwrap();

    let md = read(&f, "/docs/deck.md").await.unwrap();
    assert_eq!(md.len() as u64, CAP);
}

#[tokio::test]
async fn e2e_0089_034_text_answer_one_byte_over_the_convert() {
    let _heavy = heavy().await;
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&hashes(home.path(), CAP), CAP, 120);
    held_reset();

    let v = ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await;

    refused(&v, OVER);
    let held = answer_held();
    assert!(held <= HELD_MAX, "the answer held {held} bytes, more than {HELD_MAX}");
    clean(&f, "/docs/deck.pptx").await;
}

#[tokio::test]
async fn e2e_0089_040_binary_write_with_conversion_answer_o() {
    use crate::mcp::server::WriteBytesArgs;
    use rmcp::handler::server::wrapper::Parameters;
    let _heavy = heavy().await;
    let f = office_fixture_allowance(1_000_000_000).await;
    let home = tempfile::tempdir().unwrap();
    let svc = Arc::new(cli(&hashes(home.path(), CAP), CAP, 120));
    let server = server_with(&f, svc, OWNER);

    let out = server
        .fs_write_bytes(Parameters(WriteBytesArgs {
            mount_id: MOUNT.into(),
            path: "/docs/deck.pptx".into(),
            base64: "UEs=".into(),
            overwrite: false,
            create_parents: true,
            trigger_documentation_service: true,
        }))
        .await
        .unwrap();

    assert_ne!(out.is_error, Some(true), "{out:?}");
    let v: Value = serde_json::from_str(&text_of(&out)).unwrap();
    assert_eq!(
        v["documentation"],
        json!({"error": {"code": code::INTERNAL_ERROR, "message": OVER}}),
        "{v}"
    );
    assert!(exists(&f, "/docs/deck.pptx").await);
    assert!(!exists(&f, "/docs/deck.md").await);
}

/// The same state as `f`, with the converter `svc`, acting as `person`.
fn server_with(
    f: &Fixture,
    svc: Arc<CliDocService>,
    person: &str,
) -> crate::mcp::server::McpServer {
    let s = &f.state;
    let state = crate::state::AppState {
        config: s.config.clone(),
        admin: s.admin.clone(),
        stores: s.stores.clone(),
        safety: s.safety.clone(),
        identity: s.identity.clone(),
        editors: s.editors.clone(),
        doc_service: Some(svc),
        search: None,
    };
    crate::mcp::server::McpServer::new(Arc::new(state), person.to_string())
}

fn text_of(r: &rmcp::model::CallToolResult) -> String {
    r.content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("a text content block")
}

#[tokio::test]
async fn e2e_0089_042_converter_failure_within_the_limits_u() {
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&stand_in(home.path(), "echo 'the real cause' >&2\nexit 3\n"), CAP, 120);

    let v = ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await;

    refused(&v, "document service exited with exit code 3: the real cause");
}

#[tokio::test]
async fn e2e_0089_044_timeout_unchanged() {
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&stand_in(home.path(), "exec sleep 30\n"), CAP, 1);

    let v = ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await;

    refused(&v, "doc service timed out after 1s");
    stopped_and_cleaned(home.path());
}

#[tokio::test]
async fn e2e_0089_045_document_over_the_converter_limit_ref() {
    let f = converter_fixture().await;
    let mut big = b"PK".to_vec();
    big.resize(1_048_577, 0);
    seed(&f, "/docs/big.pptx", &big).await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&hashes(home.path(), CAP), CAP, 120);

    let v = ask(&f, &svc, OWNER, "/docs/big.pptx", false).await;

    let Err(e) = v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::INVALID_ARGUMENT);
    assert_eq!(
        e.message,
        "'/docs/big.pptx' is 1048577 bytes, over the doc_service.max_input_bytes limit of 1048576"
    );
    assert!(marker(home.path()).is_none(), "the converter must never start");
}

/// Two converter floods at once, one per project, through the assistant tool:
/// each is refused with the cap message, each held at most [`HELD_MAX`], the
/// operator reads one error line per call, nothing is left. A plain test on a
/// current thread runtime so the capture and the thread local hook see both.
#[test]
fn e2e_0089_046_two_converter_floods_at_once() {
    use crate::mcp::server::DocumentizeArgs;
    use rmcp::handler::server::wrapper::Parameters;
    let _capture = crate::logging::capture::lock_for_test();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let _heavy = heavy().await;
        let f = converter_fixture().await;
        f.seed_project("orbit", SECOND).await;
        let c = f.state.stores.client("orbit").await.unwrap();
        c.makedirs("/docs", true).await.unwrap();
        c.write_bytes_atomic("/docs/deck.pptx", b"PK").await.unwrap();
        let home = tempfile::tempdir().unwrap();
        let svc = Arc::new(cli(&stand_in(home.path(), FLOOD), CAP, 120));
        let (mia, noa) = (server_with(&f, svc.clone(), OWNER), server_with(&f, svc, SECOND));
        let args = |mount: &str| DocumentizeArgs {
            mount_id: mount.into(),
            path: "/docs/deck.pptx".into(),
            overwrite: false,
        };
        held_reset();
        crate::logging::capture::clear();

        let (a, b) = tokio::join!(
            mia.fs_documentize(Parameters(args(MOUNT))),
            noa.fs_documentize(Parameters(args("orbit")))
        );

        for out in [a.unwrap(), b.unwrap()] {
            assert_eq!(out.is_error, Some(true), "{out:?}");
            assert!(text_of(&out).contains(OVER), "{}", text_of(&out));
        }
        let all = holds();
        assert_eq!(all.len(), 2, "{all:?}");
        for h in &all {
            assert!(h.answer <= HELD_MAX, "the answer held {} bytes: {all:?}", h.answer);
        }
        let events = crate::logging::capture::events();
        let level = |e: &crate::logging::capture::CapturedEvent| e.fields["level"].clone();
        let message = |e: &crate::logging::capture::CapturedEvent| e.fields["message"].clone();
        let errors: Vec<_> = events.iter().filter(|e| level(e) == "ERROR").collect();
        assert_eq!(errors.len(), 2, "{events:?}");
        for e in errors {
            assert!(
                message(e).contains("ERR_INTERNAL_ERROR") && message(e).contains(OVER),
                "{e:?}"
            );
        }
        assert_eq!(stopped_and_cleaned(home.path()), 2, "both calls started the stand in");
        clean(&f, "/docs/deck.pptx").await;
        for mount in [MOUNT, "orbit"] {
            let c = f.state.stores.client(mount).await.unwrap();
            assert!(!c.exists("/docs/deck.md").await.unwrap(), "{mount}");
        }
    });
}

#[tokio::test]
async fn e2e_0089_047_empty_answer_unchanged() {
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&stand_in(home.path(), "exit 0\n"), CAP, 120);

    let v = ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await;

    let Err(e) = v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::INTERNAL_ERROR);
    assert!(
        e.message.starts_with("document service exited with exit code 0 but produced no output"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn e2e_0089_048_not_a_member_converter_flood() {
    let f = converter_fixture().await;
    let home = tempfile::tempdir().unwrap();
    let _svc = cli(&stand_in(home.path(), FLOOD), CAP, 120);

    let e = f.state.authorize(MOUNT, "eve@example.com").await.unwrap_err();

    assert_eq!(e.code, code::FORBIDDEN);
    assert_eq!(e.message, "'eve@example.com' is not a member of 'atlas'");
    assert!(marker(home.path()).is_none(), "the converter must never start");
}

#[tokio::test]
async fn e2e_0089_049_companion_exists_without_overwrite_co() {
    let f = converter_fixture().await;
    seed(&f, "/docs/deck.md", b"old").await;
    let home = tempfile::tempdir().unwrap();
    let svc = cli(&stand_in(home.path(), FLOOD), CAP, 120);

    let v = ask(&f, &svc, OWNER, "/docs/deck.pptx", false).await;

    let Err(e) = v else { panic!("must be refused: {v:?}") };
    assert_eq!(e.code, code::NO_CLOBBER);
    assert_eq!(e.message, "'/docs/deck.md' exists (pass overwrite=true)");
    assert_eq!(read(&f, "/docs/deck.md").await.as_deref(), Some(b"old".as_slice()));
    assert!(marker(home.path()).is_none(), "the converter must never start");
}
