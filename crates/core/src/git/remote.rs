//! The single remote pipeline: host resolution, URL validation, credential
//! supply and (eventually) the four remote operations, all in one module so
//! the security properties FR-NEW-041/042 rest on are proved once rather than
//! four times (FR-NEW-054).
//!
//! `config.rs` declares the `hosts` field on [`crate::config::GitConfig`] and
//! calls [`validate_hosts`] once from `ServerConfig::validate`. Every other read
//! of an entry's value, including resolution, lives here. `tools/git.rs` is the
//! sole caller of [`resolve_host`] for an actual clone URL: it parses the URL
//! with the `url` crate, lowercases the extracted host, and resolves it here.
//!
//! Host matching is exact only (DEC-015): no wildcard, no substring, no prefix
//! fallback. A host absent from the map is not a `Provider::Anonymous`, it is a
//! [`Result::Err`], because silently downgrading an undeclared host to anonymous
//! would be exactly the ambiguity exact matching exists to remove.
//!
//! [`validate_remote_url`] and [`clone_to_temp`] (with the sole
//! `git2::RemoteCallbacks` construction in the tree) complete the pipeline for
//! clone; [`require_remote`] is the guard push, fetch and pull (US-009 to
//! US-011) all call before ever reaching a network call, since none of those
//! three tools take a `url` argument (DEC-021): their only source for one is
//! the `origin` row [`crate::tools::git`]'s clone records via `git::db`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer, Serialize};
use tracing::Instrument;

use crate::config::GitConfig;
use crate::errors::{Result, ToolError};
use crate::git::repo::GitRepoStore;
use crate::safety::SafetyManager;

/// The raw `git.hosts` YAML shape: hostname to provider-name pairs, in the order
/// the operator wrote them, keeping every duplicate rather than collapsing it.
///
/// `serde`'s built-in `HashMap<String, String>` deserialization silently keeps
/// only the last value for a repeated YAML key, which would make FR-NEW-003
/// (reject a duplicate host) undetectable: the duplicate would already be gone
/// by the time `validate_hosts` ran. The hand-written [`Deserialize`] below
/// collects every `(key, value)` pair `serde_yaml` hands it instead of folding
/// them into a map, so a duplicate survives to be rejected explicitly.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HostMap(pub Vec<(String, String)>);

impl<'de> Deserialize<'de> for HostMap {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct HostMapVisitor;
        impl<'de> serde::de::Visitor<'de> for HostMapVisitor {
            type Value = HostMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping of hostname to provider")
            }
            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, String>()? {
                    entries.push(entry);
                }
                Ok(HostMap(entries))
            }
        }
        deserializer.deserialize_map(HostMapVisitor)
    }
}

/// Credential policy attached to a trusted git host.
///
/// `Github` also covers GitHub Enterprise Server: the credential shape and
/// `git.auth`'s validation are provider specific, not host specific (DEC-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Gitlab,
    Generic,
    Anonymous,
}

impl Provider {
    /// The four accepted `git.hosts` values, lowercase exact. Listed here once so
    /// every "unknown provider" error names the same set the parser accepts.
    pub const ALL: [&'static str; 4] = ["github", "gitlab", "generic", "anonymous"];

    /// Lowercase-exact parse: `"GitHub"` and `""` are both rejected (E2E-NEW-004,
    /// E2E-NEW-005), because a provider value is either one of the four accepted
    /// strings or the map is misconfigured.
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "github" => Some(Self::Github),
            "gitlab" => Some(Self::Gitlab),
            "generic" => Some(Self::Generic),
            "anonymous" => Some(Self::Anonymous),
            _ => None,
        }
    }
}

/// One validated `git.hosts` entry: a bare hostname paired with its credential
/// policy, immutable once boot validation has published it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEntry {
    pub host: String,
    pub provider: Provider,
}

/// The published map, keyed by the exact hostname string the operator wrote.
///
/// `OnceLock` gives the container process lifetime; the `RwLock` inside lets
/// [`validate_hosts`] replace its contents exactly once in production (at boot)
/// while keeping the storage itself immutable-after-boot in spirit: nothing but
/// `validate_hosts` ever writes to it, and no tool or route calls that function
/// (FR-NEW-005).
static HOSTS: OnceLock<RwLock<HashMap<String, Provider>>> = OnceLock::new();

fn store() -> &'static RwLock<HashMap<String, Provider>> {
    HOSTS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Reject anything that is not a bare hostname: a scheme, a path, a port, or a
/// wildcard character (FR-NEW-004). `url::Host::parse` is the DRIFT-003
/// dependency: it is what actually rejects a scheme (`"https://github.com"`
/// fails IDNA validation on the embedded `/`), a path (`"github.com/org"`) and a
/// port (`"github.com:443"`), because none of those is a syntactically valid
/// host component. It does not reject a wildcard, so that check is explicit.
fn validate_host_key(host: &str) -> Result<()> {
    if host.contains('*') || host.contains('?') {
        return Err(ToolError::invalid_argument(format!(
            "git.hosts key '{host}' contains a wildcard character: host matching is exact only, \
             one key per hostname"
        )));
    }
    url::Host::parse(host).map_err(|e| {
        ToolError::invalid_argument(format!(
            "git.hosts key '{host}' is not a bare hostname (no scheme, path, or port \
             allowed): {e}"
        ))
    })?;
    Ok(())
}

/// Validate every `git.hosts` entry and, only once every entry is valid, publish
/// the map for [`resolve_host`]. A partial map is never published: an operator
/// never gets a boot that silently accepted half of what they wrote.
pub fn validate_hosts(cfg: &GitConfig) -> Result<()> {
    let mut resolved: HashMap<String, Provider> = HashMap::new();
    for (host, provider_raw) in &cfg.hosts.0 {
        validate_host_key(host)?;
        let provider = Provider::parse(provider_raw).ok_or_else(|| {
            ToolError::invalid_argument(format!(
                "git.hosts entry '{host}' has unknown provider '{provider_raw}', expected one \
                 of {}",
                Provider::ALL.join(", ")
            ))
        })?;
        if resolved.contains_key(host) {
            return Err(ToolError::invalid_argument(format!(
                "git.hosts contains duplicate host '{host}'"
            )));
        }
        resolved.insert(host.clone(), provider);
    }
    *store().write().expect("git host map lock poisoned") = resolved;
    Ok(())
}

/// Resolve a bare hostname to its declared credential policy. `O(1)`: a single
/// hash map lookup against the map [`validate_hosts`] published, never a scan.
///
/// A host absent from the map is `Err`, not `Provider::Anonymous`: exact
/// matching means an undeclared host is undeclared, not implicitly trusted.
pub fn resolve_host(host: &str) -> Result<Provider> {
    let map = store().read().expect("git host map lock poisoned");
    map.get(host)
        .copied()
        .ok_or_else(|| ToolError::not_found(format!("host '{host}' is not declared in git.hosts")))
}

/// The `provider` field a `git.remote` tracing span and audit entry record
/// for this host (FR-NEW-072): the same lookup [`resolve_host`] performs,
/// reduced to its four string labels plus `"unknown"` for a host absent from
/// `git.hosts` (or unknowable yet, before a network capable pre-flight
/// check even resolved one). Read only, for labeling: it never gates a
/// credential decision, [`resolve_host`] still owns that.
pub fn provider_label(host: &str) -> &'static str {
    match resolve_host(host) {
        Ok(Provider::Github) => "github",
        Ok(Provider::Gitlab) => "gitlab",
        Ok(Provider::Generic) => "generic",
        Ok(Provider::Anonymous) => "anonymous",
        Err(_) => "unknown",
    }
}

/// Every declared host whose provider is not `anonymous`, sorted by host
/// ascending. The token screen's host selector (FR-NEW-038) reads `git.hosts`
/// only through this function, never `GitConfig` directly, so host resolution
/// still lives in exactly one file (the self-review checklist's "one
/// implementation" rule).
pub fn credentialed_hosts() -> Vec<String> {
    let map = store().read().expect("git host map lock poisoned");
    let mut hosts: Vec<String> =
        map.iter().filter(|(_, p)| **p != Provider::Anonymous).map(|(h, _)| h.clone()).collect();
    hosts.sort();
    hosts
}

/// Reject anything that is not a bare `git@host:path` style shorthand: no
/// `://` anywhere, and a colon that comes after a host-shaped prefix (no `/`
/// before it). This is what `url::Url::parse` would otherwise turn into some
/// unrelated parse error instead of the scheme rejection FR-NEW-041 requires
/// (E2E-NEW-147): the scp form carries an implicit `ssh` scheme that never
/// appears in the string for `url::Url` to name.
fn looks_like_scp_shorthand(raw: &str) -> bool {
    if raw.contains("://") {
        return false;
    }
    match raw.find(':') {
        Some(idx) => {
            let before = &raw[..idx];
            !before.is_empty() && !before.contains('/')
        }
        None => false,
    }
}

/// Validate a remote URL before any network call and before any audit entry is
/// written (FR-NEW-041, FR-NEW-042). Parses the URL exactly once: the returned
/// [`url::Url`] is what callers reuse for host extraction, so an operation
/// never parses its URL twice.
///
/// Rejects: any scheme other than `https` (naming the scheme, or naming the
/// implicit `ssh` of the scp shorthand); and any URL carrying a userinfo
/// component, whose error names the host only, never the URL itself, since the
/// URL is exactly what carries the leaked credential.
pub fn validate_remote_url(raw: &str) -> Result<url::Url> {
    if looks_like_scp_shorthand(raw) {
        return Err(ToolError::invalid_argument(format!(
            "remote url '{raw}' uses the scp-style shorthand for an implicit 'ssh' scheme, \
             which is not supported: only https urls are accepted"
        )));
    }
    let parsed = url::Url::parse(raw).map_err(|e| {
        ToolError::invalid_argument(format!("remote url '{raw}' is not a valid URL: {e}"))
    })?;
    if parsed.scheme() != "https" {
        return Err(ToolError::invalid_argument(format!(
            "remote url scheme '{}' is not supported: only https is accepted",
            parsed.scheme()
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        let host = parsed.host_str().unwrap_or("<unknown host>");
        return Err(ToolError::invalid_argument(format!(
            "remote url for host '{host}' must not embed credentials in the URL; store one \
             with git.auth instead"
        )));
    }
    Ok(parsed)
}

/// Build the credential-supplying callbacks every remote operation shares. The
/// sole `git2::RemoteCallbacks` construction in the tree (FR-NEW-054,
/// E2E-NEW-197): clone and push both call this one function rather than each
/// constructing their own, so the credential-supply property this proves rests
/// on one implementation, not two.
///
/// `deadline`, when set, is also wired into `transfer_progress` (FR-NEW-044,
/// DRIFT-009): a transfer already under way can observe the same deadline the
/// caller's `tokio::time::timeout` around the whole operation enforces, and
/// abort by returning `false`. This is best-effort, secondary enforcement
/// only: `transfer_progress` does not fire during connect or the TLS
/// handshake, so the outer timeout is what actually bounds a hang there.
/// Defensive second layer for FR-NEW-043: never actually observed in
/// practice, since every URL this pipeline hands to `git2` is already
/// userinfo free ([`validate_remote_url`]) and the token itself only ever
/// reaches libgit2 through `Cred::userpass_plaintext` inside a callback, not
/// as literal text libgit2 could echo back into an error string. Kept as a
/// cheap safety net rather than trusting that the absence of a known leak
/// path means no leak can ever occur: every libgit2 error message this
/// module wraps is passed through this first.
fn redact(message: String, token: Option<&str>) -> String {
    match token {
        Some(t) if !t.is_empty() && message.contains(t) => message.replace(t, "<redacted>"),
        _ => message,
    }
}

/// Everything one remote operation's tracing span and audit entry need to
/// know before it runs (FR-NEW-057, FR-NEW-072): which tool, which host,
/// which credential policy, which branch (when the operation names one), and
/// where to write the audit entry.
///
/// Built at exactly one of two places for a given call, never both: the
/// pre-flight guards in `tools/git.rs` (a failure resolved before any
/// network call, FR-NEW-007/049/070, reported through
/// [`tools::git::early_remote_failure`](crate::tools::git)) when one of them
/// fails, or the operation function itself
/// (`clone_and_import`/`push_branch`/`fetch_branch`/`pull_branch`) once every
/// guard has passed. That mutual exclusion is what keeps the span and audit
/// count at exactly one per call (FR-NEW-072), without either site needing
/// to know whether the other one already reported.
pub struct RemoteOpContext<'a> {
    pub operation: &'static str,
    /// The declared remote the operation targets (FR-MOD-101, FR-MOD-103):
    /// named in the span and in the audit detail, so two remotes on the same
    /// host stay distinguishable in the trail.
    pub remote: &'a str,
    pub host: &'a str,
    pub provider: &'a str,
    pub branch: Option<&'a str>,
    pub mount_id: &'a str,
    pub person: &'a str,
    pub safety: &'a SafetyManager,
}

/// Run one remote operation's future inside exactly one `git.remote` tracing
/// span, then record exactly one `safety.record_audit` entry from its
/// outcome (FR-NEW-057, FR-NEW-072). The single call site both properties
/// rest on.
///
/// `describe` builds the audit detail from a successful result; a failed
/// result's detail is the error's own message. Neither a token nor a
/// credentialed URL ever reaches either: every [`ToolError`] this pipeline
/// raises already excludes both (FR-NEW-041, FR-NEW-042), and `describe` is
/// supplied by each call site from response data that was never
/// token-bearing to begin with (FR-NEW-043).
pub async fn run_remote_operation<T, F>(
    op: RemoteOpContext<'_>,
    future: F,
    describe: impl FnOnce(&T) -> String,
) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    let span = tracing::info_span!(
        "git.remote",
        operation = op.operation,
        remote = op.remote,
        host = op.host,
        provider = op.provider,
        branch = op.branch.unwrap_or(""),
        outcome = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
    );
    let start = Instant::now();
    let result = future.instrument(span.clone()).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let outcome = if result.is_ok() { "ok" } else { "error" };
    span.record("outcome", outcome);
    span.record("duration_ms", duration_ms);

    let detail = match &result {
        Ok(v) => describe(v),
        Err(e) => e.message.clone(),
    };
    let branch_part = op.branch.map(|b| format!(" branch '{b}'")).unwrap_or_default();
    op.safety.record_audit(
        op.person,
        op.mount_id,
        op.operation,
        "/",
        &format!(
            "host '{}' remote '{}'{branch_part} outcome {outcome}: {detail}",
            op.host, op.remote
        ),
    );

    result
}

fn credential_callbacks(
    token: Option<String>,
    deadline: Option<Instant>,
) -> git2::RemoteCallbacks<'static> {
    let mut callbacks = git2::RemoteCallbacks::new();
    if let Some(t) = token {
        // The provider expects the token as the password; "oauth2" is the
        // conventional username for both GitHub and GitLab.
        callbacks
            .credentials(move |_url, _user, _types| git2::Cred::userpass_plaintext("oauth2", &t));
    }
    if let Some(deadline) = deadline {
        callbacks.transfer_progress(move |_progress| Instant::now() < deadline);
    }
    callbacks
}

/// Clone into a real directory: libgit2 needs a filesystem to clone into.
///
/// `deadline` is the same instant the caller's `tokio::time::timeout` around
/// this whole call is racing (FR-NEW-044): wired into `transfer_progress` so a
/// transfer already under way can also observe it (DRIFT-009). The blocking
/// OS thread this runs on is not killed by either mechanism; it keeps running
/// until its own socket or TLS operation errors out on its own, which is a
/// known, accepted limitation, not a defect this hides.
pub fn clone_to_temp(
    url: &str,
    into: &Path,
    branch: Option<&str>,
    depth: i64,
    token: Option<String>,
    deadline: Option<Instant>,
) -> Result<git2::Repository> {
    let token_for_redaction = token.clone();
    let callbacks = credential_callbacks(token, deadline);
    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(callbacks);
    if depth > 0 {
        fetch.depth(depth.min(i32::MAX as i64) as i32);
    }

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch);
    if let Some(b) = branch {
        builder.branch(b);
    }
    builder.clone(url, into).map_err(|e| {
        // The message may name the URL but never the token: git2 does not echo
        // credentials, and the token never appears in the URL we pass. Redacted
        // defensively regardless (FR-NEW-043).
        ToolError::internal(redact(
            format!("clone failed: {e} (see server logs for details)"),
            token_for_redaction.as_deref(),
        ))
    })
}

/// FR-NEW-021, FR-MOD-101, FR-MOD-103: push, fetch and pull take no `url`;
/// they resolve a remote declared for the volume, `origin` unless the caller
/// named another one. `git.remote_push`, `git.remote_fetch` and
/// `git.remote_pull` each call this before ever reaching a network operation,
/// rather than each reimplementing "does this volume have this remote": one
/// `list_remotes` query settles it, and a name that is not there fails here,
/// before any connection is opened (FR-NEW-146).
///
/// The two failure shapes are deliberately distinct. An absent `origin` stays
/// `ERR_INVALID_ARGUMENT` with the wording every caller already matches on: a
/// volume never initialized as a repository, and one initialized but never
/// cloned into, both mean "this volume has no default remote to work with".
/// Any other name the caller supplied is `ERR_NOT_FOUND`, naming the remote
/// and the volume: the caller asked for a specific remote and it does not
/// exist.
pub async fn require_remote(store: &GitRepoStore, volume_id: &str, remote: &str) -> Result<String> {
    let missing = |uninitialized: bool| {
        if remote == "origin" {
            let suffix =
                if uninitialized { ": it was never initialized as a git repository" } else { "" };
            ToolError::invalid_argument(format!(
                "volume '{volume_id}' has no origin remote{suffix}"
            ))
        } else {
            ToolError::not_found(format!(
                "remote '{remote}' is not declared for volume '{volume_id}': add it with \
                 git.remote_add, or list what is declared with git.remote_list"
            ))
        }
    };
    if !store.is_initialized(volume_id).await {
        return Err(missing(true));
    }
    let db = store.get_db(volume_id).await?;
    let remotes = db.list_remotes().await?;
    remotes
        .into_iter()
        .find(|(name, _)| name == remote)
        .map(|(_, url)| url)
        .ok_or_else(|| missing(false))
}

/// The outcome of a successful push (FR-NEW-060): the branch's new sha on the
/// remote, whether the branch was created there, and whether nothing actually
/// changed (the remote already held this sha). `created` and `up_to_date` are
/// never both true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    pub remote_sha: String,
    pub created: bool,
    pub up_to_date: bool,
    /// The sha the remote branch held immediately before this push, read from
    /// the transport's own pre-push advertisement; `None` when the branch did
    /// not exist there. FR-NEW-155/159: for a forced push this is the sha that
    /// was destroyed, and the only record of it.
    pub overwritten_sha: Option<String>,
}

/// Push `refs/heads/{branch}` to the same name on `origin_url`, from a bare
/// repository already hydrated with every local object (FR-NEW-022). Shares
/// [`credential_callbacks`] rather than building its own (FR-NEW-054): the sole
/// `git2::RemoteCallbacks` construction in the tree stays in that one
/// function, this one only calls it.
///
/// `origin_url`'s scheme is not re-validated here: it was already checked by
/// [`validate_remote_url`] when it was recorded, at clone time. This makes the
/// function callable directly, with a `file://` origin, by a test proving real
/// push mechanics against a local bare repository with no network involved,
/// exactly like [`clone_to_temp`] is callable directly with one.
///
/// `created` and `up_to_date` come from `push_negotiation`'s pre-push view of
/// the remote's current ref value, the same advertisement the transport itself
/// negotiates against, never from a local guess. The remote is authoritative
/// on fast-forwardness (FR-NEW-024): no local pre-flight check runs here, a
/// rejection is only ever recognized from `push_update_reference`'s own status
/// message, classified by [`classify_push_rejection`].
#[allow(clippy::too_many_arguments)]
pub fn push_to_remote(
    repo: &git2::Repository,
    origin_url: &str,
    branch: &str,
    remote_branch: &str,
    local_sha: &str,
    token: Option<String>,
    deadline: Option<Instant>,
    force: bool,
    expected_remote_sha: Option<&str>,
) -> Result<PushOutcome> {
    let token_for_redaction = token.clone();
    let mut remote = repo.remote_anonymous(origin_url).map_err(|e| {
        ToolError::internal(redact(
            format!("push failed to open remote: {e}"),
            token_for_redaction.as_deref(),
        ))
    })?;

    // FR-MOD-102: the two sides of the refspec are named independently, so a
    // local branch can land under another name on the remote. Everything the
    // outcome is read from (the negotiation's pre-push view, the per-ref
    // rejection status) keys off the REMOTE side, which is the ref the remote
    // actually reports on.
    let src_refname = format!("refs/heads/{branch}");
    let dst_refname = format!("refs/heads/{remote_branch}");
    let dst_for_negotiation = dst_refname.clone();
    let remote_before: Rc<RefCell<Option<git2::Oid>>> = Rc::new(RefCell::new(None));
    let before_cell = remote_before.clone();
    let rejected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let rejected_cell = rejected.clone();
    // FR-NEW-157: the sha the remote actually holds, captured only when it
    // fails to match the lease, so the caller can be told both values.
    let lease_actual: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let lease_actual_cell = lease_actual.clone();
    let lease = expected_remote_sha.map(str::to_string);
    let lease_for_callback = lease.clone();

    let mut callbacks = credential_callbacks(token, deadline);
    callbacks.push_negotiation(move |updates| {
        for u in updates {
            if u.dst_refname() == Some(dst_for_negotiation.as_str()) {
                *before_cell.borrow_mut() = Some(u.src());
                // The lease is checked HERE, against the transport's own
                // pre-push advertisement of the ref, which costs no second
                // connection and no second round trip: it is the same view
                // `created` and `up_to_date` are already read from. Returning
                // an error aborts the push before a single object or ref
                // update is sent, which is what leaves a stale-leased remote
                // byte-identical (FR-NEW-157).
                if let Some(expected) = lease_for_callback.as_deref() {
                    let actual = u.src().to_string();
                    if actual != expected {
                        *lease_actual_cell.borrow_mut() = Some(actual);
                        return Err(git2::Error::from_str("expected_remote_sha does not match"));
                    }
                }
            }
        }
        Ok(())
    });
    callbacks.push_update_reference(move |_refname, status| {
        if let Some(msg) = status {
            *rejected_cell.borrow_mut() = Some(msg.to_string());
        }
        Ok(())
    });

    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(callbacks);
    // FR-NEW-155: the leading `+` is what makes the update non-fast-forward,
    // and it is only ever built from a `force` that already carries a
    // validated lease: the two are inseparable by construction here, not by
    // convention at the call site.
    let refspec = format!("{}{src_refname}:{dst_refname}", if force { "+" } else { "" });
    if let Err(e) = remote.push(&[refspec.as_str()], Some(&mut opts)) {
        // A lease mismatch surfaces as the failure of the push the callback
        // aborted, so it is classified before anything else: the remote never
        // rejected this push, the lease did.
        if let (Some(actual), Some(expected)) = (lease_actual.borrow().clone(), lease.as_deref()) {
            return Err(lease_mismatch_error(branch, remote_branch, expected, &actual));
        }
        // The local transport (used by every test, and by a same-host push in
        // production) raises a non-fast-forward as a hard error from `push`
        // itself, before `push_update_reference` ever runs; a remote smart-HTTP
        // server instead reports it per-ref through that callback. Both paths
        // are classified the same way (FR-NEW-024): the remote's own signal is
        // authoritative regardless of which shape it arrives in.
        if e.code() == git2::ErrorCode::NotFastForward {
            return Err(non_fast_forward_error(branch));
        }
        return Err(ToolError::internal(redact(
            format!("push of branch '{branch}' failed: {e}"),
            token_for_redaction.as_deref(),
        )));
    }

    if let Some(msg) = rejected.borrow().clone() {
        return Err(classify_push_rejection(&msg, branch));
    }

    let before = *remote_before.borrow();
    let existed = before.is_some_and(|oid| !oid.is_zero());
    let previous_remote_sha = if existed { before.map(|o| o.to_string()) } else { None };
    Ok(PushOutcome {
        remote_sha: local_sha.to_string(),
        created: !existed,
        up_to_date: previous_remote_sha.as_deref() == Some(local_sha),
        overwritten_sha: previous_remote_sha,
    })
}

/// Classify a rejection reported by the remote through `push_update_reference`
/// (FR-NEW-024): a non-fast-forward rejection gets a dedicated, distinct error
/// (`ERR_NO_CLOBBER`, since the remote is refusing to let this push clobber
/// commits it holds that the volume does not); any other rejection (a
/// protected-branch message, or anything else the remote sends) surfaces the
/// remote's own text verbatim, naming the branch, under `ERR_INTERNAL_ERROR`.
/// The two are machine-distinguishable by code alone, and both are distinct
/// from the `ERR_UNAUTHENTICATED` a missing or expired credential already
/// produces before this function is ever called.
fn classify_push_rejection(status_msg: &str, branch: &str) -> ToolError {
    let lower = status_msg.to_ascii_lowercase();
    if lower.contains("non-fast-forward") || lower.contains("non fast-forward") {
        non_fast_forward_error(branch)
    } else {
        ToolError::internal(format!(
            "push of branch '{branch}' was rejected by the remote: {status_msg}"
        ))
    }
}

/// The single explicit fetch refspec (FR-NEW-062, FR-NEW-147): destination
/// side names only `refs/remotes/{remote}/*`, so an update reported by
/// libgit2's `update_tips` callback can never name anything else, in
/// particular never a local branch and never another remote's namespace.
/// `git.remote_pull` reuses [`fetch_from_remote`] itself, and therefore this
/// function, rather than writing its own copy: pull's first step is exactly
/// this fetch (DEC-012).
pub fn fetch_refspec(remote: &str) -> String {
    format!("+refs/heads/*:refs/remotes/{remote}/*")
}

/// One remote-tracking ref [`fetch_from_remote`] updated: `old_sha` is `None`
/// when the ref was newly created (FR-NEW-050).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRefUpdate {
    pub ref_name: String,
    pub old_sha: Option<String>,
    pub new_sha: String,
}

/// The outcome of a successful fetch: every `refs/remotes/origin/*` ref that
/// changed, the branch names ([`RemoteHead`](git2::RemoteHead) short names
/// under `refs/heads/`) the remote currently advertises, and how many objects
/// were downloaded. `advertised_branches` is read before the fetch itself runs
/// (DEC-037): comparing it against what a caller already has under
/// `refs/remotes/origin/*` is what lets `git.remote_fetch` tell a branch
/// deleted upstream (absent from this list) from one merely unchanged (still
/// listed, `update_tips` just never fired for it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    pub refs_updated: Vec<FetchRefUpdate>,
    pub advertised_branches: Vec<String>,
    pub objects_fetched: i64,
}

/// Fetch objects and update `refs/remotes/origin/*` from `origin_url`
/// (FR-NEW-026), using [`FETCH_REFSPEC`], the single explicit refspec
/// (FR-NEW-062). Tag following is disabled and pruning is forced off, so the
/// only refs this can ever create or move are under `refs/remotes/origin/`,
/// and a branch removed upstream is left exactly where it was rather than
/// deleted (FR-NEW-056, DEC-037). Shares [`credential_callbacks`] rather than
/// building its own (FR-NEW-054): the sole `git2::RemoteCallbacks`
/// construction in the tree stays in that one function, this one calls it
/// twice, once to list the remote's advertisement, once for the fetch itself.
///
/// `refs/heads/*` and every volume file are untouched (FR-NEW-027): this
/// function never writes either, and the bare repository it runs against has
/// no working tree to begin with.
///
/// `origin_url`'s scheme is not re-validated here, exactly like
/// [`push_to_remote`]: it was already checked by [`validate_remote_url`] when
/// it was recorded, at clone time. This makes the function callable directly,
/// with a `file://` origin, by a test proving real fetch mechanics against a
/// local bare repository with no network involved.
pub fn fetch_from_remote(
    repo: &git2::Repository,
    remote: &str,
    origin_url: &str,
    token: Option<String>,
    deadline: Option<Instant>,
) -> Result<FetchOutcome> {
    let token_for_redaction = token.clone();
    let mut git_remote = repo
        .remote_anonymous(origin_url)
        .map_err(|e| connection_error(origin_url, &e, token_for_redaction.as_deref()))?;

    // The remote's own advertisement, read before the fetch itself runs
    // (DEC-037): the caller diffs this against its prior
    // `refs/remotes/origin/*` view to find what went stale.
    let advertised_branches: Vec<String> = {
        let list_callbacks = credential_callbacks(token.clone(), deadline);
        let conn = git_remote
            .connect_auth(git2::Direction::Fetch, Some(list_callbacks), None)
            .map_err(|e| connection_error(origin_url, &e, token_for_redaction.as_deref()))?;
        conn.list()
            .map_err(|e| connection_error(origin_url, &e, token_for_redaction.as_deref()))?
            .iter()
            .filter_map(|h| h.name().strip_prefix("refs/heads/").map(str::to_string))
            .collect()
        // `conn` drops here, disconnecting before the real fetch reconnects.
    };

    let updates: Rc<RefCell<Vec<FetchRefUpdate>>> = Rc::new(RefCell::new(Vec::new()));
    let updates_cell = updates.clone();
    let mut callbacks = credential_callbacks(token, deadline);
    callbacks.update_tips(move |refname, old, new| {
        updates_cell.borrow_mut().push(FetchRefUpdate {
            ref_name: refname.to_string(),
            old_sha: if old.is_zero() { None } else { Some(old.to_string()) },
            new_sha: new.to_string(),
        });
        true
    });

    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(callbacks);
    // FR-NEW-062: no tag ever follows this fetch, and pruning stays off so a
    // ref absent from `advertised_branches` is reported, never deleted here.
    opts.download_tags(git2::AutotagOption::None);
    opts.prune(git2::FetchPrune::Off);

    git_remote
        .fetch(&[fetch_refspec(remote).as_str()], Some(&mut opts), None)
        .map_err(|e| connection_error(origin_url, &e, token_for_redaction.as_deref()))?;

    let objects_fetched = git_remote.stats().received_objects() as i64;
    let refs_updated = updates.borrow().clone();

    Ok(FetchOutcome { refs_updated, advertised_branches, objects_fetched })
}

/// Wrap a git2 failure reaching the network with the host named explicitly
/// (E2E-NEW-091), distinct from a credential failure: that one is always
/// `ERR_UNAUTHENTICATED`, raised earlier, before any network call, by
/// `require_valid_credential`. This is always `ERR_INTERNAL_ERROR`, so the two
/// are machine-distinguishable by code alone.
fn connection_error(origin_url: &str, e: &git2::Error, token: Option<&str>) -> ToolError {
    let host = extract_host(origin_url);
    ToolError::internal(redact(format!("fetch from host '{host}' failed: {e}"), token))
}

/// The bare hostname of a URL, or the URL itself when it does not parse: the
/// one place [`connection_error`] and [`timeout_error`] both get a host to
/// name from a URL that was already validated by [`validate_remote_url`] at
/// record time, so a parse failure here cannot actually occur in production.
pub(crate) fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}

/// Governs every remote operation's network round trip: clone, push, fetch
/// and pull all pass through this one function (FR-NEW-044), so the deadline
/// is proved once, not four times. `future` is the `on_git_thread`-wrapped
/// blocking call; the caller must have already acquired the per-repository
/// `write_lock` itself, before this function is ever invoked, never inside
/// the blocking closure `future` wraps (DRIFT-009). When the deadline fires,
/// `tokio::time::timeout` drops `future` and this returns `Err` immediately;
/// the caller's own `?` then unwinds its stack frame, dropping the
/// `write_lock` guard it is holding right there, which is what actually
/// releases the lock (E2E-NEW-156) even though the orphaned blocking OS
/// thread the network call was running on is NOT stopped: `git2` 0.20.4
/// exposes no cancellation handle, so that thread keeps running until its own
/// socket or TLS operation eventually errors out at the OS layer. That is a
/// known, accepted limitation, not a defect this function hides.
pub async fn with_remote_deadline<T>(
    timeout_secs: u64,
    host: &str,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(Duration::from_secs(timeout_secs), future).await {
        Ok(result) => result,
        Err(_) => Err(timeout_error(host, timeout_secs)),
    }
}

/// FR-NEW-049: the frozen code and message prefix for a remote deadline
/// expiry, naming the host.
fn timeout_error(host: &str, timeout_secs: u64) -> ToolError {
    ToolError::internal(format!(
        "remote timeout: host '{host}' did not respond within {timeout_secs}s"
    ))
}

/// The dedicated, distinct identity for a non-fast-forward refusal
/// (FR-NEW-024): named, distinguishable (`ERR_NO_CLOBBER`) from both a
/// credential failure (`ERR_UNAUTHENTICATED`, raised earlier by
/// `require_valid_credential`, before this function is ever called) and a
/// generic rejection ([`classify_push_rejection`]'s `ERR_INTERNAL_ERROR`
/// branch). Shared by both places a local transport push can surface this: a
/// hard `git2::Error` with `ErrorCode::NotFastForward` from `remote.push`
/// itself (what every test in this tree observes, local transport being the
/// only kind reachable without a real server), and a per-ref rejection
/// reported through `push_update_reference`'s status message (what a real
/// smart-HTTP remote sends).
fn non_fast_forward_error(branch: &str) -> ToolError {
    ToolError::invalid_argument(format!(
        "push refused: not a fast-forward: branch '{branch}', pass force with \
         expected_remote_sha to overwrite the remote branch"
    ))
}

/// FR-NEW-157: the lease failed, so the remote moved since the caller last
/// looked. Deliberately a different message identity from
/// [`non_fast_forward_error`] (it does not start with that prefix): the two
/// mean different things, and a caller reacting to them must be able to tell
/// a remote that moved from a push that was simply never a fast-forward.
/// Both shas are named, and labelled, because acting on this error means
/// re-reading the remote and deciding whether the actual sha is still
/// something worth overwriting.
fn lease_mismatch_error(
    branch: &str,
    remote_branch: &str,
    expected: &str,
    actual: &str,
) -> ToolError {
    ToolError::invalid_argument(format!(
        "push refused: the remote moved: branch '{branch}' targets '{remote_branch}' on the \
         remote, expected {expected}, actual {actual}; nothing was overwritten, re-read the \
         remote tip before forcing again"
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::GitConfig;

    /// Serializes every test that publishes to or reads from the process-global
    /// [`HOSTS`] map. `validate_hosts` replaces the whole map on every call, and
    /// `cargo test` runs unit tests in parallel threads within one process, so
    /// two tests touching the map concurrently would race on each other's
    /// contents. Tests that only assert a boot failure never reach the publish
    /// step, so they need no lock: nothing was ever written.
    pub(crate) fn lock_for_test() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn cfg_with_hosts(pairs: &[(&str, &str)]) -> GitConfig {
        let mut c = GitConfig::default();
        c.hosts.0 = pairs.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        c
    }

    const REFERENCE_MAP: &[(&str, &str)] = &[
        ("github.com", "github"),
        ("github.ibm.com", "github"),
        ("gitlab.acme.corp", "gitlab"),
        ("git.acme.internal", "generic"),
        ("public.example.org", "anonymous"),
    ];

    /// E2E-NEW-001: a valid map boots and every host resolves to its declared
    /// provider class.
    #[test]
    fn e2e_new_001_valid_map_boots_and_resolves_every_provider_class() {
        let _guard = lock_for_test();
        let cfg = cfg_with_hosts(REFERENCE_MAP);
        validate_hosts(&cfg).expect("the reference map is valid");

        assert_eq!(resolve_host("github.com").unwrap(), Provider::Github);
        assert_eq!(resolve_host("github.ibm.com").unwrap(), Provider::Github);
        assert_eq!(resolve_host("gitlab.acme.corp").unwrap(), Provider::Gitlab);
        assert_eq!(resolve_host("git.acme.internal").unwrap(), Provider::Generic);
        assert_eq!(resolve_host("public.example.org").unwrap(), Provider::Anonymous);
    }

    /// E2E-NEW-002: two hosts may share one provider and resolve independently.
    #[test]
    fn e2e_new_002_two_hosts_may_share_one_provider() {
        let _guard = lock_for_test();
        let cfg = cfg_with_hosts(&[("github.com", "github"), ("github.ibm.com", "github")]);
        validate_hosts(&cfg).expect("two hosts on the same provider is valid");

        assert_eq!(resolve_host("github.com").unwrap(), Provider::Github);
        assert_eq!(resolve_host("github.ibm.com").unwrap(), Provider::Github);
    }

    /// E2E-NEW-003: an unknown provider value fails boot, naming the host and
    /// listing the accepted values.
    #[test]
    fn e2e_new_003_unknown_provider_value_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.ibm.com", "githib")]);
        let e = validate_hosts(&cfg).expect_err("githib is not an accepted provider");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github.ibm.com"), "{}", e.message);
        for name in Provider::ALL {
            assert!(e.message.contains(name), "{name} should be listed: {}", e.message);
        }
    }

    /// E2E-NEW-004: provider values are lowercase-exact, so `GitHub` fails boot.
    #[test]
    fn e2e_new_004_uppercase_provider_value_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com", "GitHub")]);
        let e = validate_hosts(&cfg).expect_err("provider values are lowercase-exact");
        assert!(e.message.contains("github.com"), "{}", e.message);
    }

    /// E2E-NEW-005: an empty provider value fails boot, naming the host.
    #[test]
    fn e2e_new_005_empty_provider_value_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com", "")]);
        let e = validate_hosts(&cfg).expect_err("an empty provider value is invalid");
        assert!(e.message.contains("github.com"), "{}", e.message);
    }

    /// E2E-NEW-006: a duplicate host fails boot naming the host, and neither
    /// entry is silently chosen (nothing is published: the loop errors before
    /// `store()` is ever written for this map).
    #[test]
    fn e2e_new_006_duplicate_host_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com", "github"), ("github.com", "anonymous")]);
        let e = validate_hosts(&cfg).expect_err("github.com appears twice");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github.com"), "{}", e.message);
        assert!(e.message.contains("duplicate"), "{}", e.message);
    }

    /// E2E-NEW-007: a host with a scheme fails boot, naming the offending key.
    #[test]
    fn e2e_new_007_host_with_a_scheme_fails_boot() {
        let cfg = cfg_with_hosts(&[("https://github.com", "github")]);
        let e = validate_hosts(&cfg).expect_err("a scheme is not a bare hostname");
        assert!(e.message.contains("https://github.com"), "{}", e.message);
    }

    /// E2E-NEW-008: a host with a path fails boot, naming the offending key.
    #[test]
    fn e2e_new_008_host_with_a_path_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com/org", "github")]);
        let e = validate_hosts(&cfg).expect_err("a path is not a bare hostname");
        assert!(e.message.contains("github.com/org"), "{}", e.message);
    }

    /// E2E-NEW-009: a host with a port fails boot, naming the key.
    #[test]
    fn e2e_new_009_host_with_a_port_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com:443", "github")]);
        let e = validate_hosts(&cfg).expect_err("a port is not a bare hostname");
        assert!(e.message.contains("github.com:443"), "{}", e.message);
    }

    /// E2E-NEW-010: a wildcard host fails boot, naming the key, and the message
    /// does not suggest wildcards are supported (DEC-015).
    #[test]
    fn e2e_new_010_wildcard_host_fails_boot() {
        let cfg = cfg_with_hosts(&[("*.acme.corp", "gitlab")]);
        let e = validate_hosts(&cfg).expect_err("wildcards are not supported");
        assert!(e.message.contains("*.acme.corp"), "{}", e.message);
        assert!(
            !e.message.to_ascii_lowercase().contains("supported"),
            "the message must not suggest wildcards are supported: {}",
            e.message
        );
    }

    /// E2E-NEW-013: an empty map boots, and a host is then undeclared rather
    /// than implicitly anonymous.
    #[test]
    fn e2e_new_013_empty_map_boots() {
        let _guard = lock_for_test();
        let cfg = cfg_with_hosts(&[]);
        validate_hosts(&cfg).expect("an empty git.hosts map is valid");

        let e = resolve_host("github.com").expect_err("github.com was never declared");
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
        assert!(e.message.contains("github.com"), "{}", e.message);
    }

    /// E2E-NEW-014: an absent `hosts` key behaves exactly like an empty map,
    /// because `GitConfig` derives `Default` for it.
    #[test]
    fn e2e_new_014_absent_map_section_boots() {
        let _guard = lock_for_test();
        let cfg = GitConfig::default();
        assert!(cfg.hosts.0.is_empty(), "an absent hosts key must default to empty");
        validate_hosts(&cfg).expect("a config with no hosts key is valid");

        let e = resolve_host("github.com").expect_err("no host was ever declared");
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
    }

    /// E2E-NEW-017: no tool in the registry accepts input that changes a
    /// host-to-provider entry. `git.hosts` is config-only surface: this
    /// enumerates the real, currently registered tool set (the families this
    /// story touches nothing in) and asserts none of them exposes a `hosts`
    /// property.
    #[test]
    fn e2e_new_017_no_registered_tool_can_mutate_the_host_map() {
        let mut reg = crate::mcp::ToolRegistry::new();
        crate::tools::register_fs(&mut reg);
        crate::tools::admin::register(&mut reg);
        crate::tools::git::register(&mut reg);
        crate::tools::git_auth::register(&mut reg);

        for &name in reg.names() {
            let tool = reg.resolve(name).expect("just listed");
            let schema = tool.schema.input_schema();
            assert!(
                schema.get("properties").and_then(|p| p.get("hosts")).is_none(),
                "tool '{name}' must not accept a 'hosts' property"
            );
            assert!(
                !name.to_ascii_lowercase().contains("host"),
                "tool '{name}' name must not reference hosts"
            );
        }
    }

    /// E2E-NEW-111: no request a web screen could issue changes a
    /// host-to-provider entry. No such screen exists in this crate yet (it is
    /// out of this story's scope), and git has no REST surface at all
    /// (`api/dataplane.rs` carries none), so this is verified by scanning the
    /// one REST surface that exists for any reference to `git.hosts`.
    #[test]
    fn e2e_new_111_the_screen_cannot_modify_the_host_map() {
        let dataplane = include_str!("../api/dataplane.rs");
        assert!(
            !dataplane.contains("git.hosts") && !dataplane.contains("git::remote"),
            "api/dataplane.rs must not read or write git.hosts: git has no REST surface"
        );
    }

    /// E2E-NEW-224: validation and resolution live in one module. This proves
    /// existence and reachability by calling both functions through their full
    /// path, and proves `config.rs` calls `validate_hosts` exactly once and
    /// inspects no entry itself by scanning its own source.
    #[test]
    fn e2e_new_224_validation_and_resolution_live_in_one_module() {
        let _guard = lock_for_test();
        let cfg = GitConfig::default();
        crate::git::remote::validate_hosts(&cfg).expect("an empty map validates");
        let _ = crate::git::remote::resolve_host("git.unknown.test");

        let config_src = include_str!("../config.rs");
        assert!(config_src.contains("pub hosts:"), "GitConfig must declare the hosts field");
        assert_eq!(
            config_src.matches("validate_hosts(").count(),
            1,
            "config.rs must call validate_hosts exactly once"
        );
        for needle in [".hosts.0.iter()", ".hosts.0.contains", "Provider::parse"] {
            assert!(
                !config_src.contains(needle),
                "config.rs must not inspect a git.hosts entry itself (found {needle:?})"
            );
        }
    }

    /// E2E-NEW-225: boot validation still fires from `ServerConfig::validate`.
    #[test]
    fn e2e_new_225_boot_validation_fires_from_server_config_validate() {
        let mut cfg = crate::config::ServerConfig::default();
        cfg.git.hosts.0 = vec![("github.ibm.com".to_string(), "githib".to_string())];
        let e = cfg.validate().expect_err("an unknown provider must fail ServerConfig::validate");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github.ibm.com"), "{}", e.message);
    }

    /// E2E-NEW-226: a valid map passes validation from the same path, and every
    /// host resolves through `git::remote::resolve_host` afterwards.
    #[test]
    fn e2e_new_226_a_valid_map_passes_validation_from_the_same_path() {
        let _guard = lock_for_test();
        let mut cfg = crate::config::ServerConfig::default();
        cfg.git.hosts.0 =
            REFERENCE_MAP.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        cfg.validate().expect("the reference map is valid");

        for (host, provider) in [
            ("github.com", Provider::Github),
            ("github.ibm.com", Provider::Github),
            ("gitlab.acme.corp", Provider::Gitlab),
            ("git.acme.internal", Provider::Generic),
            ("public.example.org", Provider::Anonymous),
        ] {
            assert_eq!(crate::git::remote::resolve_host(host).unwrap(), provider);
        }
    }

    // ── FR-NEW-041/042: URL validation ─────────────────────────────────────

    /// E2E-NEW-142: a plain HTTPS URL passes validation.
    #[test]
    fn e2e_new_142_an_https_url_is_accepted() {
        let parsed = validate_remote_url("https://github.ibm.com/o/r.git").unwrap();
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("github.ibm.com"));
    }

    /// E2E-NEW-143: an `ssh://` URL is rejected, naming the scheme.
    #[test]
    fn e2e_new_143_an_ssh_url_is_rejected() {
        let e = validate_remote_url("ssh://git@github.com/o/r.git").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("ssh"), "got {}", e.message);
    }

    /// E2E-NEW-144: a `git://` URL is rejected, naming the scheme.
    #[test]
    fn e2e_new_144_a_git_url_is_rejected() {
        let e = validate_remote_url("git://github.com/o/r.git").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("git"), "got {}", e.message);
    }

    /// E2E-NEW-145: a `file://` URL is rejected, whether or not it names a path
    /// a caller might otherwise read off the server's own disk.
    #[test]
    fn e2e_new_145_a_file_url_is_rejected() {
        for url in ["file:///etc/passwd", "file:///tmp/repo"] {
            let e = validate_remote_url(url).unwrap_err();
            assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT, "{url}");
            assert!(e.message.contains("file"), "{url}: got {}", e.message);
        }
    }

    /// E2E-NEW-146: a plain `http://` URL is rejected: a token must never
    /// travel unencrypted.
    #[test]
    fn e2e_new_146_a_plain_http_url_is_rejected() {
        let e = validate_remote_url("http://github.com/o/r.git").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("http"), "got {}", e.message);
    }

    /// E2E-NEW-147: the scp-style shorthand is rejected as not HTTPS, and is
    /// never silently reinterpreted as a bare hostname (which would otherwise
    /// surface as some unrelated parse error rather than a scheme rejection).
    #[test]
    fn e2e_new_147_the_scp_shorthand_is_rejected() {
        let e = validate_remote_url("git@github.com:org/repo.git").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.to_ascii_lowercase().contains("ssh"), "got {}", e.message);
    }

    /// E2E-NEW-148: a URL carrying userinfo is rejected, and the error names
    /// neither the credential nor the full URL, only the host.
    #[test]
    fn e2e_new_148_a_url_carrying_userinfo_is_rejected() {
        let e = validate_remote_url("https://alice:ghp_secret@github.com/o/r.git").unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(!e.message.contains("ghp_secret"), "got {}", e.message);
        assert!(!e.message.contains("alice"), "got {}", e.message);
        assert!(
            !e.message.contains("https://alice:ghp_secret@github.com/o/r.git"),
            "the full URL must not appear: got {}",
            e.message
        );
        assert!(e.message.contains("github.com"), "the host must be named: got {}", e.message);
    }

    // ── FR-NEW-021: the no-origin guard ─────────────────────────────────────

    fn origin_test_config(root: &std::path::Path) -> std::sync::Arc<crate::config::ServerConfig> {
        let mut c = crate::config::ServerConfig::default();
        c.infra.meta.dir = root.join("state/volumes").display().to_string();
        c.infra.blob.dir = root.join("state/blobs").display().to_string();
        c.infra.admin.path = root.join("state/admin.db").display().to_string();
        c.git.enabled = true;
        std::sync::Arc::new(c)
    }

    /// E2E-NEW-080: a volume never initialized as a repository fails cleanly,
    /// with no panic and no partial state.
    #[tokio::test]
    async fn e2e_new_080_a_never_initialized_volume_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            GitRepoStore::new(origin_test_config(dir.path()), crate::storage::test_registry());
        let e = require_remote(&store, "proj", "origin").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.to_ascii_lowercase().contains("origin"), "got {}", e.message);
    }

    /// E2E-NEW-075 / E2E-NEW-089 / E2E-NEW-124: push, fetch and pull don't exist
    /// yet (US-009/010/011), so each is exercised here through the exact shared
    /// function those stories will call: a volume from `git.init` with no
    /// recorded remote has no `origin` to resolve.
    #[tokio::test]
    async fn e2e_new_075_089_124_an_initialized_volume_with_no_remote_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            GitRepoStore::new(origin_test_config(dir.path()), crate::storage::test_registry());
        store.init_repo("proj").await.unwrap();
        let e = require_remote(&store, "proj", "origin").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.to_ascii_lowercase().contains("origin"), "got {}", e.message);
    }

    /// A volume with a recorded `origin` resolves it: the guard only rejects
    /// absence, never a present remote.
    #[tokio::test]
    async fn require_remote_returns_the_stored_url_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            GitRepoStore::new(origin_test_config(dir.path()), crate::storage::test_registry());
        let entry = store.init_repo("proj").await.unwrap();
        entry.db.add_remote("origin", "https://example.test/o/r.git").await.unwrap();
        let url = require_remote(&store, "proj", "origin").await.unwrap();
        assert_eq!(url, "https://example.test/o/r.git");
    }

    // ── FR-NEW-054: one implementation ──────────────────────────────────────

    /// E2E-NEW-197: `git2::RemoteCallbacks` is constructed in exactly one file,
    /// this one; `tools/git.rs` builds no `RemoteCallbacks` of its own.
    #[test]
    fn e2e_new_197_the_remote_pipeline_exists_once() {
        let remote_src = include_str!("remote.rs");
        let tools_git_src = include_str!("../tools/git.rs");
        // Count only the non-test portion of this very file, since the test
        // below necessarily mentions the construction it is looking for.
        let production_src = remote_src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(
            production_src.matches("RemoteCallbacks::new()").count(),
            1,
            "remote.rs must construct RemoteCallbacks exactly once"
        );
        assert_eq!(
            tools_git_src.matches("RemoteCallbacks").count(),
            0,
            "tools/git.rs must not construct a RemoteCallbacks of its own"
        );
    }

    // ── FR-NEW-024: push rejections are classified into distinct identities ──

    /// E2E-NEW-076 / E2E-NEW-181 (unit half): a status message containing
    /// "non-fast-forward" is classified with a dedicated identity, the frozen
    /// prefix (FR-NEW-049), naming the branch and stating force is not
    /// supported.
    #[test]
    fn a_non_fast_forward_status_gets_a_dedicated_identity() {
        let err = classify_push_rejection("non-fast-forward", "main");
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(err.message.starts_with("push refused: not a fast-forward"), "got {}", err.message);
        assert!(err.message.contains("main"), "got {}", err.message);
        assert!(err.message.to_ascii_lowercase().contains("force"), "got {}", err.message);
    }

    /// E2E-NEW-079: a protected-branch style rejection surfaces the remote's own
    /// message verbatim, naming the branch, and is NOT classified as
    /// non-fast-forward.
    #[test]
    fn e2e_new_079_a_protected_branch_rejection_surfaces_the_remotes_reason() {
        let msg = "GH006: Protected branch update failed for refs/heads/main.";
        let err = classify_push_rejection(msg, "main");
        assert_ne!(err.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(err.message.contains("main"), "must name the branch: {}", err.message);
        assert!(err.message.contains(msg), "must surface the remote's own text: {}", err.message);
    }

    /// E2E-NEW-077 (unit half): the non-fast-forward identity and a generic
    /// remote rejection are machine-distinguishable from each other.
    #[test]
    fn non_fast_forward_and_any_other_rejection_are_distinguishable() {
        let ff = classify_push_rejection("non-fast-forward", "main");
        let other = classify_push_rejection("GH006: Protected branch update failed.", "main");
        assert_eq!(ff.code, crate::errors::code::INVALID_ARGUMENT);
        assert_ne!(ff.code, other.code);
    }
}
