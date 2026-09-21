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
//! clone; [`require_origin`] is the guard push, fetch and pull (US-009 to
//! US-011) all call before ever reaching a network call, since none of those
//! three tools take a `url` argument (DEC-021): their only source for one is
//! the `origin` row [`crate::tools::git`]'s clone records via `git::db`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::GitConfig;
use crate::errors::{Result, ToolError};
use crate::git::repo::GitRepoStore;

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
fn credential_callbacks(token: Option<String>) -> git2::RemoteCallbacks<'static> {
    let mut callbacks = git2::RemoteCallbacks::new();
    if let Some(t) = token {
        // The provider expects the token as the password; "oauth2" is the
        // conventional username for both GitHub and GitLab.
        callbacks
            .credentials(move |_url, _user, _types| git2::Cred::userpass_plaintext("oauth2", &t));
    }
    callbacks
}

/// Clone into a real directory: libgit2 needs a filesystem to clone into.
pub fn clone_to_temp(
    url: &str,
    into: &Path,
    branch: Option<&str>,
    depth: i64,
    token: Option<String>,
) -> Result<git2::Repository> {
    let callbacks = credential_callbacks(token);
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
        // credentials, and the token never appears in the URL we pass.
        ToolError::internal(format!("clone failed: {e} (see server logs for details)"))
    })
}

/// FR-NEW-021: push, fetch and pull take no `url`; they resolve the stored
/// `origin`. `git.remote_push`, `git.remote_fetch` and `git.remote_pull`
/// (US-009 to US-011) each call this before ever reaching a network
/// operation, rather than each reimplementing "does this volume have an
/// origin". A volume never initialized as a repository and one initialized
/// but never cloned into both fail the same way: there is no `origin` row to
/// resolve.
pub async fn require_origin(store: &GitRepoStore, volume_id: &str) -> Result<String> {
    if !store.is_initialized(volume_id).await {
        return Err(ToolError::invalid_argument(format!(
            "volume '{volume_id}' has no origin remote: it was never initialized as a git \
             repository"
        )));
    }
    let db = store.get_db(volume_id).await?;
    let remotes = db.list_remotes().await?;
    remotes.into_iter().find(|(name, _)| name == "origin").map(|(_, url)| url).ok_or_else(|| {
        ToolError::invalid_argument(format!("volume '{volume_id}' has no origin remote"))
    })
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
pub fn push_to_remote(
    repo: &git2::Repository,
    origin_url: &str,
    branch: &str,
    local_sha: &str,
    token: Option<String>,
) -> Result<PushOutcome> {
    let mut remote = repo
        .remote_anonymous(origin_url)
        .map_err(|e| ToolError::internal(format!("push failed to open remote: {e}")))?;

    let refname = format!("refs/heads/{branch}");
    let dst_for_negotiation = refname.clone();
    let remote_before: Rc<RefCell<Option<git2::Oid>>> = Rc::new(RefCell::new(None));
    let before_cell = remote_before.clone();
    let rejected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let rejected_cell = rejected.clone();

    let mut callbacks = credential_callbacks(token);
    callbacks.push_negotiation(move |updates| {
        for u in updates {
            if u.dst_refname() == Some(dst_for_negotiation.as_str()) {
                *before_cell.borrow_mut() = Some(u.src());
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
    let refspec = format!("{refname}:{refname}");
    if let Err(e) = remote.push(&[refspec.as_str()], Some(&mut opts)) {
        // The local transport (used by every test, and by a same-host push in
        // production) raises a non-fast-forward as a hard error from `push`
        // itself, before `push_update_reference` ever runs; a remote smart-HTTP
        // server instead reports it per-ref through that callback. Both paths
        // are classified the same way (FR-NEW-024): the remote's own signal is
        // authoritative regardless of which shape it arrives in.
        if e.code() == git2::ErrorCode::NotFastForward {
            return Err(non_fast_forward_error(branch));
        }
        return Err(ToolError::internal(format!("push of branch '{branch}' failed: {e}")));
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

/// The single explicit fetch refspec (FR-NEW-062): destination side names only
/// `refs/remotes/origin/*`, so an update reported by libgit2's `update_tips`
/// callback can never name anything else. `git.remote_pull` (US-011) reuses
/// [`fetch_from_remote`] itself, and therefore this constant, rather than
/// writing its own copy: pull's first step is exactly this fetch (DEC-012).
pub const FETCH_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

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
    origin_url: &str,
    token: Option<String>,
) -> Result<FetchOutcome> {
    let mut remote =
        repo.remote_anonymous(origin_url).map_err(|e| connection_error(origin_url, &e))?;

    // The remote's own advertisement, read before the fetch itself runs
    // (DEC-037): the caller diffs this against its prior
    // `refs/remotes/origin/*` view to find what went stale.
    let advertised_branches: Vec<String> = {
        let list_callbacks = credential_callbacks(token.clone());
        let conn = remote
            .connect_auth(git2::Direction::Fetch, Some(list_callbacks), None)
            .map_err(|e| connection_error(origin_url, &e))?;
        conn.list()
            .map_err(|e| connection_error(origin_url, &e))?
            .iter()
            .filter_map(|h| h.name().strip_prefix("refs/heads/").map(str::to_string))
            .collect()
        // `conn` drops here, disconnecting before the real fetch reconnects.
    };

    let updates: Rc<RefCell<Vec<FetchRefUpdate>>> = Rc::new(RefCell::new(Vec::new()));
    let updates_cell = updates.clone();
    let mut callbacks = credential_callbacks(token);
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

    remote
        .fetch(&[FETCH_REFSPEC], Some(&mut opts), None)
        .map_err(|e| connection_error(origin_url, &e))?;

    let objects_fetched = remote.stats().received_objects() as i64;
    let refs_updated = updates.borrow().clone();

    Ok(FetchOutcome { refs_updated, advertised_branches, objects_fetched })
}

/// Wrap a git2 failure reaching the network with the host named explicitly
/// (E2E-NEW-091), distinct from a credential failure: that one is always
/// `ERR_UNAUTHENTICATED`, raised earlier, before any network call, by
/// `require_valid_credential`. This is always `ERR_INTERNAL_ERROR`, so the two
/// are machine-distinguishable by code alone.
fn connection_error(origin_url: &str, e: &git2::Error) -> ToolError {
    let host = url::Url::parse(origin_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| origin_url.to_string());
    ToolError::internal(format!("fetch from host '{host}' failed: {e}"))
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
    ToolError::no_clobber(format!(
        "push of branch '{branch}' was refused: non-fast-forward, and force is not supported"
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
        let e = require_origin(&store, "proj").await.unwrap_err();
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
        let e = require_origin(&store, "proj").await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.to_ascii_lowercase().contains("origin"), "got {}", e.message);
    }

    /// A volume with a recorded `origin` resolves it: the guard only rejects
    /// absence, never a present remote.
    #[tokio::test]
    async fn require_origin_returns_the_stored_url_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            GitRepoStore::new(origin_test_config(dir.path()), crate::storage::test_registry());
        let entry = store.init_repo("proj").await.unwrap();
        entry.db.add_remote("origin", "https://example.test/o/r.git").await.unwrap();
        let url = require_origin(&store, "proj").await.unwrap();
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

    /// E2E-NEW-076 (unit half): a status message containing "non-fast-forward"
    /// is classified with a dedicated identity, naming the branch and stating
    /// force is not supported.
    #[test]
    fn a_non_fast_forward_status_gets_a_dedicated_identity() {
        let err = classify_push_rejection("non-fast-forward", "main");
        assert_eq!(err.code, crate::errors::code::NO_CLOBBER);
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
        assert_ne!(err.code, crate::errors::code::NO_CLOBBER);
        assert!(err.message.contains("main"), "must name the branch: {}", err.message);
        assert!(err.message.contains(msg), "must surface the remote's own text: {}", err.message);
    }

    /// E2E-NEW-077 (unit half): the non-fast-forward identity and a generic
    /// remote rejection are machine-distinguishable from each other.
    #[test]
    fn non_fast_forward_and_any_other_rejection_are_distinguishable() {
        let ff = classify_push_rejection("non-fast-forward", "main");
        let other = classify_push_rejection("GH006: Protected branch update failed.", "main");
        assert_eq!(ff.code, crate::errors::code::NO_CLOBBER);
        assert_ne!(ff.code, other.code);
    }
}
