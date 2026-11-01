//! Wire level tests of the git fetch negotiation (SPEC-0067).
//!
//! The contract of the negotiation is a reference: for a request, the service answers like git's
//! own server (`git upload-pack --stateless-rpc`) for the same project data, except for the
//! declared differences of the spec. So these tests do not hand enumerate protocol cases: they
//! send a request to the service over HTTP, run the same bytes through the reference program on
//! a bare copy of the same project, and compare the acknowledgement lines line by line and the
//! objects as a set. Real `git` clients cover the user journeys.
//!
//! Needs the `git` program and loopback networking; a missing `git` fails the test, it does not
//! skip it (same rule as `full_stack_e2e`). No external service: the server runs in process on
//! SQLite and a local blob directory.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use mcp_fs_core::app;
use mcp_fs_core::config::ServerConfig;
use mcp_fs_core::keys;
use serde_json::{Value, json};

const ADMIN: &str = "neg-admin@mcp-fs.test";
const STRANGER: &str = "neg-stranger@mcp-fs.test";
/// rmcp answers 406 to a `/mcp` POST whose `Accept` lacks both media types.
const MCP_ACCEPT: &str = "application/json, text/event-stream";
/// The options the service offers: the reference program runs with the same ones.
const CAPS: &str = "multi_ack_detailed side-band-64k ofs-delta";

// ── the in process server ────────────────────────────────────────────────────

/// The state every test of this binary shares: two servers over one state directory, one
/// refusing anonymous reads and one allowing them. The git repository store of the service is
/// process wide and fixed by the first server built (`GitRepoStore::shared`), so servers with
/// separate directories cannot coexist in one process; they run on a runtime of their own so
/// that no test's runtime ending takes them down. Projects are named per test.
struct Fixture {
    closed: String,
    open: String,
    key: PathBuf,
}

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

fn fixture() -> &'static Fixture {
    FIXTURE.get_or_init(|| {
        require_git();
        let root = std::env::temp_dir().join(format!("mcpfs-neg-e2e-{}", std::process::id()));
        remove_stale_roots(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        let (private_key, public_key) = keys::write_keypair(&root).expect("keypair");
        let (tx, rx) = std::sync::mpsc::channel();
        let key = private_key.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("fixture runtime");
            rt.block_on(async move {
                let mut bases = Vec::new();
                for anonymous_read in [false, true] {
                    let mut c = ServerConfig::default();
                    c.auth.jwt.public_key_path = public_key.display().to_string();
                    c.auth.admins = vec![ADMIN.to_string()];
                    c.infra.meta.dir = root.join("state/volumes").display().to_string();
                    c.infra.blob.dir = root.join("state/blobs").display().to_string();
                    c.infra.admin.path = root.join("state/admin.db").display().to_string();
                    c.git.enabled = true;
                    c.git.anonymous_read = anonymous_read;
                    let router = app::build(c).await.expect("app::build");
                    let listener =
                        tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
                    let port = listener.local_addr().expect("addr").port();
                    bases.push(format!("http://127.0.0.1:{port}"));
                    tokio::spawn(
                        async move { axum::serve(listener, router).await.expect("server") },
                    );
                }
                tx.send(bases).expect("send bases");
                std::future::pending::<()>().await;
            });
        });
        let bases = rx.recv().expect("fixture servers");
        Fixture { closed: bases[0].clone(), open: bases[1].clone(), key }
    })
}

/// Fixture directories left by earlier runs of this binary (a static is never dropped, so the
/// run itself cannot clean up).
fn remove_stale_roots(current: &Path) {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("mcpfs-neg-e2e-") && e.path() != current {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

struct Server {
    base: String,
    key: PathBuf,
    http: reqwest::Client,
}

impl Server {
    async fn start(anonymous_read: bool) -> Server {
        let f = fixture();
        Server {
            base: if anonymous_read { f.open.clone() } else { f.closed.clone() },
            key: f.key.clone(),
            http: reqwest::Client::new(),
        }
    }

    fn token(&self, who: &str) -> String {
        keys::mint_token_from_file(&self.key, who, keys::DEFAULT_ISSUER, keys::DEFAULT_CLAIM, 3600)
            .expect("token")
    }

    fn url(&self, project: &str) -> String {
        format!("{}/git/{project}", self.base)
    }

    /// One MCP `tools/call` as the admin, decoded from the server's SSE framing.
    async fn call_tool(&self, name: &str, arguments: Value) -> Value {
        let token = self.token(ADMIN);
        let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "git-negotiation-e2e", "version": "0.0"}}});
        let resp = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&token)
            .header("Accept", MCP_ACCEPT)
            .json(&init)
            .send()
            .await
            .expect("initialize");
        assert!(resp.status().is_success(), "initialize: {}", resp.status());
        let session = resp
            .headers()
            .get("Mcp-Session-Id")
            .expect("session id")
            .to_str()
            .expect("ascii")
            .to_string();
        let ok = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&token)
            .header("Accept", MCP_ACCEPT)
            .header("Mcp-Session-Id", &session)
            .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .send()
            .await
            .expect("initialized");
        assert!(ok.status().is_success());
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        let resp = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&token)
            .header("Accept", MCP_ACCEPT)
            .header("Mcp-Session-Id", &session)
            .json(&body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let status = resp.status();
        let text = resp.text().await.expect("body");
        assert!(status.is_success(), "{name}: http {status}: {text}");
        let envelope: Value = text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
            .next_back()
            .unwrap_or_else(|| panic!("{name}: not JSON: {text}"));
        assert!(envelope.get("error").is_none(), "{name}: {envelope}");
        let content = envelope["result"]["content"][0]["text"].as_str().expect("content");
        serde_json::from_str(content).expect("content json")
    }

    /// A project the admin owns, with its git repository initialised and no commit.
    async fn project(&self, id: &str) {
        self.call_tool("admin.create_project", json!({"project_id": id, "owner": ADMIN})).await;
        self.call_tool("git.init", json!({"mount_id": id})).await;
    }

    /// POST a hand written request; returns the status and the body.
    async fn upload_pack(&self, project: &str, token: Option<&str>, body: &[u8]) -> (u16, Vec<u8>) {
        let mut rb = self
            .http
            .post(format!("{}/git-upload-pack", self.url(project)))
            .header("Content-Type", "application/x-git-upload-pack-request")
            .body(body.to_vec());
        if let Some(t) = token {
            rb = rb.bearer_auth(t);
        }
        let resp = rb.send().await.expect("upload-pack");
        let status = resp.status().as_u16();
        (status, resp.bytes().await.expect("body").to_vec())
    }
}

fn unique_project() -> String {
    format!("demo-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn require_git() {
    let ok =
        Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    assert!(ok, "the real `git` program is required on PATH for git_negotiation_e2e");
}

// ── the git program ──────────────────────────────────────────────────────────

fn git(dir: &Path, token: Option<&str>, args: &[&str]) -> Output {
    let mut c = Command::new("git");
    c.current_dir(dir);
    if let Some(t) = token {
        c.arg("-c").arg(format!("http.extraHeader=Authorization: Bearer {t}"));
    }
    c.args(args).output().unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

fn git_ok(dir: &Path, token: Option<&str>, args: &[&str]) -> String {
    let out = git(dir, token, args);
    assert!(
        out.status.success(),
        "git {args:?} in {dir:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A working repository on `main` with the identity git needs to commit.
fn work_repo(dir: &Path) {
    git_ok(dir, None, &["init", "-q", "-b", "main"]);
    git_ok(dir, None, &["config", "user.email", "neg@mcp-fs.test"]);
    git_ok(dir, None, &["config", "user.name", "neg"]);
}

/// Commit `file` with `content` on the current branch; returns the commit id.
fn commit_file(dir: &Path, file: &str, content: &str, message: &str) -> String {
    if let Some(parent) = Path::new(file).parent() {
        std::fs::create_dir_all(dir.join(parent)).expect("mkdir");
    }
    std::fs::write(dir.join(file), content).expect("write");
    git_ok(dir, None, &["add", "-A"]);
    git_ok(dir, None, &["commit", "-q", "-m", message]);
    git_ok(dir, None, &["rev-parse", "HEAD"])
}

/// Everything a baseline test needs: a server, a project holding `n` commits (c1 to cn,
/// `f<i>.txt` added by ci) pushed from a working repository.
struct Env {
    server: Server,
    /// Unique per test: the servers of this binary share one state directory.
    project: String,
    work: tempfile::TempDir,
    shas: Vec<String>,
}

impl Env {
    /// A project with no commit yet, and an empty working repository.
    async fn new(anonymous_read: bool) -> Env {
        let server = Server::start(anonymous_read).await;
        let project = unique_project();
        server.project(&project).await;
        let work = tempfile::tempdir().expect("work dir");
        work_repo(work.path());
        Env { server, project, work, shas: Vec::new() }
    }

    async fn with_history(n: usize, anonymous_read: bool) -> Env {
        let mut env = Env::new(anonymous_read).await;
        for i in 1..=n {
            let sha = commit_file(
                env.work.path(),
                &format!("f{i}.txt"),
                &format!("file {i}\n"),
                &format!("c{i}"),
            );
            env.shas.push(sha);
        }
        env.push();
        env
    }

    fn admin(&self) -> String {
        self.server.token(ADMIN)
    }

    /// Push the working repository's `main` to the project.
    fn push(&self) {
        let t = self.admin();
        git_ok(
            self.work.path(),
            Some(&t),
            &["push", "-q", &self.server.url(&self.project), "main:refs/heads/main"],
        );
    }

    /// A bare copy of the project's data: what the reference program serves.
    fn bare_copy(&self) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("bare dir");
        let t = self.admin();
        let target = dir.path().join("ref.git");
        git_ok(
            dir.path(),
            Some(&t),
            &["clone", "-q", "--bare", &self.server.url(&self.project), target.to_str().unwrap()],
        );
        dir
    }

    /// The project's branches and tags as `git ls-remote` shows them.
    fn refs(&self) -> String {
        let t = self.admin();
        git_ok(self.work.path(), Some(&t), &["ls-remote", &self.server.url(&self.project)])
    }
}

/// A working copy of the project made with a real `git clone`, ready to commit locally.
fn clone_copy(env: &Env, token: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("copy dir");
    let copy = dir.path().join("copy");
    git_ok(
        dir.path(),
        token,
        &["clone", "-q", &env.server.url(&env.project), copy.to_str().unwrap()],
    );
    git_ok(&copy, None, &["config", "user.email", "neg@mcp-fs.test"]);
    git_ok(&copy, None, &["config", "user.name", "neg"]);
    (dir, copy)
}

fn fetch(copy: &Path, token: Option<&str>) {
    git_ok(copy, token, &["fetch", "-q", "--tags"]);
}

/// `git fsck` on a copy: the integrity check of the spec.
fn assert_intact(copy: &Path) {
    git_ok(copy, None, &["fsck", "--strict"]);
}

fn count_commits(copy: &Path, rev: &str) -> usize {
    git_ok(copy, None, &["rev-list", "--count", rev]).parse().expect("count")
}

fn holds_any(answer: &Answer, ids: &[&str]) -> bool {
    ids.iter().any(|i| answer.objects.contains(*i))
}

// ── requests and answers ─────────────────────────────────────────────────────

fn pkt(line: &str) -> Vec<u8> {
    format!("{:04x}{line}", line.len() + 4).into_bytes()
}

/// The lines a git client sends: the wants (options on the first), a flush, the haves, then
/// the end marker or a flush.
fn request(wants: &[&str], haves: &[&str], done: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, w) in wants.iter().enumerate() {
        let caps = if i == 0 { format!(" {CAPS}") } else { String::new() };
        out.extend(pkt(&format!("want {w}{caps}\n")));
    }
    out.extend(b"0000");
    for h in haves {
        out.extend(pkt(&format!("have {h}\n")));
    }
    if done {
        out.extend(pkt("done\n"));
    } else {
        out.extend(b"0000");
    }
    out
}

/// What a client observes of an answer: the acknowledgement lines in order and the set of
/// objects received. The packing and the order of the objects are not compared.
#[derive(Debug, PartialEq)]
struct Answer {
    lines: Vec<String>,
    objects: BTreeSet<String>,
}

fn parse_answer(body: &[u8]) -> Answer {
    let (mut lines, mut pack) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < body.len() {
        assert!(i + 4 <= body.len(), "trailing fragment of {} bytes", body.len() - i);
        let len = usize::from_str_radix(std::str::from_utf8(&body[i..i + 4]).expect("hex"), 16)
            .expect("pkt length");
        if len == 0 {
            i += 4;
            continue;
        }
        assert!(len >= 4 && i + len <= body.len(), "bad pkt length {len} at {i}");
        let data = &body[i + 4..i + len];
        i += len;
        match data.first() {
            Some(1) => pack.extend_from_slice(&data[1..]),
            Some(2) | None => {}
            Some(3) => {
                panic!("fatal error on side-band 3: {}", String::from_utf8_lossy(&data[1..]))
            }
            Some(_) => {
                let line = String::from_utf8_lossy(data).trim_end().to_string();
                assert!(line.starts_with("ACK ") || line == "NAK", "unexpected line: {line:?}");
                lines.push(line);
            }
        }
    }
    Answer { lines, objects: pack_objects(&pack) }
}

/// The object ids a pack carries, listed by indexing it into a scratch repository.
fn pack_objects(pack: &[u8]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if pack.is_empty() {
        return set;
    }
    assert!(pack.len() >= 32 && &pack[..4] == b"PACK", "not a pack: {} bytes", pack.len());
    // a pack header says how many objects follow; an empty one carries none
    if u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]) == 0 {
        return set;
    }
    let dir = tempfile::tempdir().expect("scratch");
    let repo = git2::Repository::init_bare(dir.path()).expect("scratch repo");
    let odb = repo.odb().expect("odb");
    let mut writer = odb.packwriter().expect("packwriter");
    writer.write_all(pack).expect("write pack");
    writer.commit().expect("index pack");
    odb.foreach(|oid| {
        set.insert(oid.to_string());
        true
    })
    .expect("list objects");
    set
}

/// The reference answer: git's own server on a bare copy of the project data.
fn reference_answer(bare: &Path, request: &[u8]) -> Answer {
    let mut child = Command::new("git")
        .args(["upload-pack", "--stateless-rpc", bare.join("ref.git").to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git upload-pack");
    let mut stdin = child.stdin.take().expect("stdin");
    let body = request.to_vec();
    // written from a thread: a large request must not deadlock against the output pipe
    let writer = std::thread::spawn(move || stdin.write_all(&body));
    let out = child.wait_with_output().expect("reference output");
    let _ = writer.join();
    assert!(
        out.status.success(),
        "the reference program failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    parse_answer(&out.stdout)
}

/// Our answer to the same request, over HTTP as the admin.
async fn our_answer(env: &Env, request: &[u8]) -> Answer {
    our_answer_as(env, request, Some(&env.admin())).await
}

async fn our_answer_as(env: &Env, request: &[u8], token: Option<&str>) -> Answer {
    let (status, body) = env.server.upload_pack(&env.project, token, request).await;
    assert_eq!(status, 200, "upload-pack status");
    parse_answer(&body)
}

fn assert_same_answer(ours: &Answer, reference: &Answer) {
    assert_eq!(ours.lines, reference.lines, "acknowledgement lines differ from the reference");
    assert_eq!(ours.objects, reference.objects, "object sets differ from the reference");
}

/// The audit log and the trash of the project: reads must leave both as they were.
async fn side_state(env: &Env) -> (Value, Value) {
    (
        env.server.call_tool("fs.audit_log", json!({"mount_id": env.project, "limit": 100})).await,
        env.server.call_tool("fs.trash_list", json!({"mount_id": env.project})).await,
    )
}

// ── baseline guards (behavior that must not change) ─────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn e2e_001_clone_of_a_four_commit_project() {
    let env = Env::with_history(4, false).await;
    let (refs0, side0) = (env.refs(), side_state(&env).await);
    let into = tempfile::tempdir().unwrap();
    let copy = into.path().join("copy");
    git_ok(
        into.path(),
        Some(&env.admin()),
        &["clone", "-q", &env.server.url(&env.project), copy.to_str().unwrap()],
    );
    assert_eq!(git_ok(&copy, None, &["rev-list", "--count", "HEAD"]), "4");
    assert_eq!(git_ok(&copy, None, &["rev-parse", "HEAD"]), env.shas[3]);
    git_ok(&copy, None, &["fsck", "--strict"]);

    // the same clone as a hand written request: same objects as the reference
    let bare = env.bare_copy();
    let req = request(&[&env.shas[3]], &[], true);
    let ours = our_answer(&env, &req).await;
    assert_eq!(ours.objects.len(), 12, "4 commits, 4 folder listings, 4 files");
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(env.refs(), refs0, "a clone changes nothing");
    assert_eq!(side_state(&env).await, side0, "audit log and trash are unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_012_wants_and_nothing_else() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let tip = &env.shas[5];
    // the reference sends nothing without an end marker; the service keeps sending everything
    let ours = our_answer(&env, &request(&[tip], &[], false)).await;
    let everything = reference_answer(bare.path(), &request(&[tip], &[], true));
    // a clone's closing line is a bare NAK for the reference too
    assert_eq!(ours.lines, everything.lines);
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert_eq!(ours.objects, everything.objects);
    assert_eq!(ours.objects.len(), 6 * 3, "6 commits, 6 folder listings, 6 files");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_013_empty_project() {
    let server = Server::start(false).await;
    let project = unique_project();
    server.project(&project).await;
    let token = server.token(ADMIN);
    let into = tempfile::tempdir().unwrap();
    let copy = into.path().join("copy");
    let out =
        git(into.path(), Some(&token), &["clone", &server.url(&project), copy.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("You appear to have cloned an empty repository."),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!git(&copy, None, &["rev-parse", "--verify", "HEAD"]).status.success(), "no commit");
    assert_eq!(
        git_ok(into.path(), Some(&token), &["ls-remote", &server.url(&project)]),
        "",
        "the project is unchanged: still no ref"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_015_wanted_object_unknown_to_the_project() {
    let env = Env::with_history(4, false).await;
    let before = env.refs();
    let unknown = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let ours = our_answer(&env, &request(&[unknown], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_016_wanted_object_that_is_not_an_id() {
    let env = Env::with_history(4, false).await;
    let before = env.refs();
    let ours = our_answer(&env, &request(&["xyz"], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_017_empty_request() {
    let env = Env::with_history(4, false).await;
    let before = env.refs();
    let (status, body) = env.server.upload_pack(&env.project, Some(&env.admin()), b"").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"0000".to_vec(), "a request that wants nothing gets a bare flush");
    assert_eq!(env.refs(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_019_a_non_member_is_refused() {
    let env = Env::with_history(4, false).await;
    let stranger = env.server.token(STRANGER);
    let (status, body) = env
        .server
        .upload_pack(&env.project, Some(&stranger), &request(&[&env.shas[3]], &[], true))
        .await;
    assert_eq!(status, 403, "{}", String::from_utf8_lossy(&body));
    assert!(!body.windows(4).any(|w| w == b"PACK"), "no object is sent");
    let out = git(env.work.path(), Some(&stranger), &["ls-remote", &env.server.url(&env.project)]);
    assert!(!out.status.success(), "the git program is refused too");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_020_anonymous_caller_refused_when_anonymous_reading_is_off() {
    let env = Env::with_history(4, false).await;
    let (status, body) =
        env.server.upload_pack(&env.project, None, &request(&[&env.shas[3]], &[], true)).await;
    assert_eq!(status, 401, "{}", String::from_utf8_lossy(&body));
    assert!(!body.windows(4).any(|w| w == b"PACK"), "no object is sent");
    let out = git(env.work.path(), None, &["ls-remote", &env.server.url(&env.project)]);
    assert!(!out.status.success(), "the git program is refused too");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_022_reads_change_nothing() {
    let env = Env::with_history(4, false).await;
    let (refs0, side0) = (env.refs(), side_state(&env).await);
    let into = tempfile::tempdir().unwrap();
    let copy = into.path().join("copy");
    // a clone, then a fetch with nothing new: nothing changes
    git_ok(
        into.path(),
        Some(&env.admin()),
        &["clone", "-q", &env.server.url(&env.project), copy.to_str().unwrap()],
    );
    git_ok(&copy, Some(&env.admin()), &["fetch", "-q"]);
    assert_eq!(env.refs(), refs0, "branches and tags are identical");
    assert_eq!(side_state(&env).await, side0, "no audit entry, trash unchanged");
    // 2 new commits arrive by a push; the fetch that takes them changes nothing more
    let c5 = commit_file(env.work.path(), "g5.txt", "five\n", "c5");
    let c6 = commit_file(env.work.path(), "g6.txt", "six\n", "c6");
    env.push();
    let refs1 = env.refs();
    assert!(refs1.contains(&c6) && !refs1.contains(&c5), "only the new tip is listed: {refs1}");
    let side1 = side_state(&env).await;
    git_ok(&copy, Some(&env.admin()), &["fetch", "-q"]);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_eq!(env.refs(), refs1, "branches and tags are identical");
    assert_eq!(side_state(&env).await, side1, "the fetch added no audit entry and left the trash");
}

// ── the end of the exchange (US-0136) ───────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn e2e_002_fetch_of_two_new_commits() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let c5 = commit_file(env.work.path(), "c.txt", "c\n", "c5");
    let c6 = commit_file(env.work.path(), "f1.txt", "changed\n", "c6");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(count_commits(&copy, "origin/main"), 6);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_intact(&copy);

    // what the client sent: wants c6, holds c4, ends the negotiation
    let bare = env.bare_copy();
    let req = request(&[&c6], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 6, "commit, folder listing and file, for c5 and for c6");
    let old: Vec<&str> = env.shas.iter().map(String::as_str).collect();
    assert!(!holds_any(&ours, &old) && ours.objects.contains(&c5));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_003_fetch_with_nothing_new() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let refs0 = env.refs();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(count_commits(&copy, "HEAD"), 4);
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let req = request(&[c4], &[c4], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.objects.is_empty(), "nothing to send");
    assert_eq!(ours.lines, vec![format!("ACK {c4} common"), format!("ACK {c4}")]);
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_004_the_copy_holds_commits_the_project_lacks() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let mut locals = Vec::new();
    for i in 1..=3 {
        locals.push(commit_file(&copy, &format!("l{i}.txt"), "local\n", &format!("l{i}")));
    }
    let c5 = commit_file(env.work.path(), "d.txt", "d\n", "c5");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c5);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "HEAD"]), locals[2], "local commits are kept");
    assert_intact(&copy);

    let bare = env.bare_copy();
    let haves: Vec<&str> =
        [locals[2].as_str(), locals[1].as_str(), locals[0].as_str(), env.shas[3].as_str()].to_vec();
    let req = request(&[&c5], &haves, true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 3, "the commit, its folder listing, d.txt");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_005_forty_local_commits_two_in_common() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    for i in 1..=36 {
        commit_file(&copy, &format!("local{i}.txt"), "local\n", &format!("local {i}"));
    }
    assert_eq!(count_commits(&copy, "HEAD"), 40);
    let c5 = commit_file(env.work.path(), "d.txt", "d\n", "c5");
    let c6 = commit_file(env.work.path(), "e.txt", "e\n", "c6");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_eq!(count_commits(&copy, "HEAD"), 40, "the 40 commits are kept");
    assert_intact(&copy);

    // the last exchange as the client sends it: every commit it holds, newest first
    let held = git_ok(&copy, None, &["rev-list", "HEAD"]);
    let haves: Vec<&str> = held.lines().collect();
    let bare = env.bare_copy();
    let req = request(&[&c6], &haves, true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 6, "only c5 and c6");
    assert!(ours.objects.contains(&c5) && ours.objects.contains(&c6));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_006_merge_history_with_one_side_held() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let w = env.work.path();
    git_ok(w, None, &["checkout", "-q", "-b", "side", &env.shas[2]]);
    let s1 = commit_file(w, "side.txt", "side\n", "s1");
    git_ok(w, None, &["checkout", "-q", "main"]);
    git_ok(w, None, &["merge", "-q", "--no-ff", "-m", "m1", "side"]);
    let m1 = git_ok(w, None, &["rev-parse", "HEAD"]);
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), m1);
    assert_intact(&copy);

    let bare = env.bare_copy();
    let req = request(&[&m1], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.objects.contains(&s1) && ours.objects.contains(&m1));
    let old: Vec<&str> = env.shas.iter().map(String::as_str).collect();
    assert!(!holds_any(&ours, &old), "no commit of c1 to c4 is sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_007_a_file_deleted_long_ago_and_added_back() {
    let env = Env::new(false).await;
    let w = env.work.path();
    commit_file(w, "old.txt", "old\n", "c1");
    git_ok(w, None, &["rm", "-q", "old.txt"]);
    git_ok(w, None, &["commit", "-q", "-m", "c2"]);
    commit_file(w, "other.txt", "other\n", "c3");
    let c3 = git_ok(w, None, &["rev-parse", "HEAD"]);
    env.push();
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let c4 = commit_file(w, "old.txt", "old\n", "c4");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c4);
    assert_intact(&copy);

    // whichever objects git sends here, the service sends the same ones
    let bare = env.bare_copy();
    let req = request(&[&c4], &[&c3], true);
    assert_same_answer(&our_answer(&env, &req).await, &reference_answer(bare.path(), &req));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_011_twelve_unknown_held_commits() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let unknown: Vec<String> = (0..12).map(|i| format!("{:040x}", 0x1000 + i)).collect();
    let haves: Vec<&str> = unknown.iter().map(String::as_str).collect();
    let req = request(&[&env.shas[5]], &haves, true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no acknowledgement for an unknown id");
    assert_eq!(ours.objects.len(), 18, "everything reachable from c6");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_014_a_held_id_that_is_not_a_valid_id() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let tip = &env.shas[5];
    let ours = our_answer(&env, &request(&[tip], &["not-an-id"], true)).await;
    // as if the id had not been stated: the reference answer to the request without it
    let reference = reference_answer(bare.path(), &request(&[tip], &[], true));
    assert_same_answer(&ours, &reference);
    assert!(ours.lines.iter().all(|l| !l.contains("not-an-id")));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_021_anonymous_caller_allowed_when_anonymous_reading_is_on() {
    let env = Env::with_history(4, true).await;
    let (_d, copy) = clone_copy(&env, None);
    let c5 = commit_file(env.work.path(), "c.txt", "c\n", "c5");
    let c6 = commit_file(env.work.path(), "f1.txt", "changed\n", "c6");
    env.push();
    fetch(&copy, None);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_eq!(count_commits(&copy, "origin/main"), 6);
    let bare = env.bare_copy();
    let req = request(&[&c6], &[&env.shas[3]], true);
    let ours = our_answer_as(&env, &req, None).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 6, "exactly the objects of c5 and c6");
    assert!(ours.objects.contains(&c5));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_029_held_id_of_a_file_version() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let blob = git_ok(env.work.path(), None, &["rev-parse", &format!("{c4}:f1.txt")]);
    let ours = our_answer(&env, &request(&[c4], &[&blob], true)).await;
    let reference = reference_answer(bare.path(), &request(&[c4], &[], true));
    assert_eq!(
        ours.lines,
        vec!["NAK".to_string()],
        "the reference would acknowledge it, the service does not"
    );
    assert!(ours.lines.iter().all(|l| !l.contains(&blob)));
    assert_eq!(ours.objects, reference.objects, "every object reachable from c4");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_032_held_id_with_only_ten_characters() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let short = &env.shas[3][..10];
    let ours = our_answer(&env, &request(&[&env.shas[5]], &[short], true)).await;
    let reference = reference_answer(bare.path(), &request(&[&env.shas[5]], &[], true));
    assert_same_answer(&ours, &reference);
    assert!(ours.lines.iter().all(|l| !l.contains(short)));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_033_held_id_of_a_folder_listing() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let tree = git_ok(env.work.path(), None, &["rev-parse", &format!("{c4}^{{tree}}")]);
    let ours = our_answer(&env, &request(&[c4], &[&tree], true)).await;
    let reference = reference_answer(bare.path(), &request(&[c4], &[], true));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert_eq!(ours.objects, reference.objects, "every object reachable from c4");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_034_held_id_of_an_annotated_tag() {
    let env = Env::with_history(4, false).await;
    let w = env.work.path();
    let c5 = commit_file(w, "d.txt", "d\n", "c5");
    env.push();
    // the reference copy is taken before the tag is pushed: a clone that has to carry an
    // annotated tag is the subject of a later story
    let bare = env.bare_copy();
    git_ok(w, None, &["tag", "-a", "-m", "first release", "v1", &env.shas[3]]);
    git_ok(w, Some(&env.admin()), &["push", "-q", &env.server.url(&env.project), "v1"]);
    let tag = git_ok(w, None, &["rev-parse", "v1"]);
    let ours = our_answer(&env, &request(&[&c5], &[&tag], true)).await;
    let reference = reference_answer(bare.path(), &request(&[&c5], &[], true));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.lines.iter().all(|l| !l.contains(&tag)));
    assert_eq!(ours.objects, reference.objects, "every object reachable from c5");
}

// ── the rounds that do not end the negotiation (US-0137) ─────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn e2e_010_negotiation_stopped_before_the_end() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let req = request(&[&env.shas[5]], &[&env.shas[2], &env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.objects.is_empty(), "no object data before the end");
    // c6 reaches the held commits, so git tells the client it has enough; the service does too
    assert_eq!(
        ours.lines,
        vec![
            format!("ACK {} common", env.shas[2]),
            format!("ACK {} common", env.shas[3]),
            format!("ACK {} ready", env.shas[3]),
            "NAK".to_string(),
        ]
    );
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_041_unusable_held_ids_without_the_end_of_the_negotiation() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let unknown: Vec<String> = (0..12).map(|i| format!("{:040x}", 0x1000 + i)).collect();
    let haves: Vec<&str> = unknown.iter().map(String::as_str).collect();
    let req = request(&[&env.shas[5]], &haves, false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_042_held_id_of_a_file_version_without_the_end_of_the_negotiation() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let c4 = &env.shas[3];
    let blob = git_ok(env.work.path(), None, &["rev-parse", &format!("{c4}:f1.txt")]);
    let ours = our_answer(&env, &request(&[&env.shas[5]], &[&blob], false)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "the reference would acknowledge the id");
    assert!(ours.lines.iter().all(|l| !l.contains(&blob)));
    assert!(ours.objects.is_empty(), "no object data before the end");
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_043_malformed_held_id_without_the_end_of_the_negotiation() {
    let env = Env::with_history(6, false).await;
    let ours = our_answer(&env, &request(&[&env.shas[5]], &["not-an-id"], false)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert!(ours.lines.iter().all(|l| !l.contains("not-an-id")));
}

/// Beyond the story's tests: the two cases where git withholds `ready` (the rule of DEC-004).
#[tokio::test(flavor = "multi_thread")]
async fn ready_is_withheld_like_git_does() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let unknown = format!("{:040x}", 0x1234);
    // an id the project never held, beside a held one
    let req = request(&[&env.shas[5]], &[&unknown, &env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().all(|l| !l.ends_with("ready")));
    // a want older than the held commit: it does not reach it
    let req = request(&[&env.shas[3]], &[&env.shas[5]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().all(|l| !l.ends_with("ready")));
    // the same held commit and a want that reaches it: ready
    let req = request(&[&env.shas[5]], &[&env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().any(|l| l.ends_with("ready")));
    // an unknown id met after a held commit that already satisfies the want: git answers
    // `ready` with that id, right where it meets it
    let req = request(&[&env.shas[5]], &[&env.shas[3], &unknown], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines[1], format!("ACK {unknown} ready"));
    // a want that is a folder listing leads to no commit: it does not hold `ready` back
    let tree = git_ok(env.work.path(), None, &["rev-parse", &format!("{}^{{tree}}", env.shas[5])]);
    let req = request(&[&tree], &[&env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().any(|l| l.ends_with("ready")));
    // and the same exchange with the end marker keeps the `ready` of the unknown id
    let req = request(&[&env.shas[5]], &[&env.shas[3], &unknown], true);
    assert_same_answer(&our_answer(&env, &req).await, &reference_answer(bare.path(), &req));
}
