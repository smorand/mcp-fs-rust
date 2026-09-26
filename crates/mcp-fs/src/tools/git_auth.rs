//! `git.auth*` tools: OAuth device flow for GitHub and GitLab.
//!
//! Port of the C# `Tools/GitAuthTools.cs`. Registered only when `git.enabled`.
//!
//! No project is involved, so the only gate is an authenticated caller: a token
//! belongs to the person, not to a mount. `git.auth` returns as soon as the
//! provider hands out a user code, then a background task polls the token
//! endpoint and stores the result in the [`OAuthTokenStore`], which is the same
//! store `git.remote_clone` reads from (hence the process singleton below).

use crate::config::ServerConfig;
use crate::errors::{Result, ToolError};
use crate::git::oauth::device_flow::{DeviceCode, DeviceFlowClient, HttpDeviceFlowClient};
use crate::git::oauth::scopes::{PrAccess, pr_capability};
use crate::git::oauth::store::OAuthTokenStore;
use crate::mcp::registry::{ToolCtx, handler};
use crate::mcp::{ToolRegistry, ToolSchema};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The bound `git.token_set` enforces on a seeded token (DEC-033): a
/// denial-of-service bound, matching `TextKey`'s length ceiling on the
/// persisted column (`crates/mcp-fs/src/git/oauth/persistence.rs:33-34`).
const MAX_TOKEN_LEN: usize = 8192;

/// A provider that returns `interval: 0` must not turn the poll loop into a spin.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Process wide token store, built once from the config. `git.auth` writes it and
/// `git.remote_clone` reads it, so they must be the same instance; the error is
/// cached too, because a malformed `MCPFS_TOKEN_KEY` must fail loudly every time
/// instead of silently downgrading to memory only storage.
/// A `tokio::sync::OnceCell` rather than a `OnceLock`: building the store now
/// loads the encrypted rows through an async driver, so initialisation awaits.
static TOKENS: tokio::sync::OnceCell<Result<Arc<OAuthTokenStore>>> =
    tokio::sync::OnceCell::const_new();

/// Process wide device flow client. Reuses one `reqwest` connection pool.
static FLOW: OnceLock<Result<Arc<dyn DeviceFlowClient>>> = OnceLock::new();

pub async fn token_store(
    config: &ServerConfig,
    registry: &crate::storage::RelationalRegistry,
) -> Result<Arc<OAuthTokenStore>> {
    TOKENS
        .get_or_init(|| async { OAuthTokenStore::from_env(config, registry).await.map(Arc::new) })
        .await
        .clone()
}

fn device_flow(config: &ServerConfig) -> Result<Arc<dyn DeviceFlowClient>> {
    FLOW.get_or_init(|| {
        HttpDeviceFlowClient::new(config.git.clone())
            .map(|c| Arc::new(c) as Arc<dyn DeviceFlowClient>)
    })
    .clone()
}

/// Register the three `git.auth*` tools.
pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, None, None);
}

/// Registration with injected dependencies, for tests: a token store that is not
/// the process singleton and a fake device flow that never reaches the network.
pub fn register_with(
    reg: &mut ToolRegistry,
    tokens: Option<Arc<OAuthTokenStore>>,
    flow: Option<Arc<dyn DeviceFlowClient>>,
) {
    let (t, f) = (tokens.clone(), flow);
    reg.add(
        ToolSchema::new(
            "git.auth",
            "Start OAuth device flow for GitHub or GitLab. Returns user_code and verification_uri.",
        )
        .req_str("provider", "OAuth provider: github or gitlab.")
        .opt_str_null(
            "host",
            "Host declared in git.hosts to authorize against; omit to use the provider's \
             canonical public host (github.com or gitlab.com).",
        )
        .opt_str_null("instance_url", "Optional self-hosted instance URL (e.g. GitLab Enterprise).")
        .destructive(false)
        .read_only(false)
        .idempotent(false)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (t, f) = (t.clone(), f.clone());
            async move {
                let provider = a.str("provider")?;
                let host = a.opt_str("host");
                let instance_url = a.opt_str("instance_url");
                let tokens = resolve_tokens(&ctx, t).await?;
                let flow = match f {
                    Some(f) => f,
                    None => device_flow(&ctx.state.config)?,
                };
                auth(&ctx, &provider, host, instance_url, tokens, flow).await
            }
        }),
    );

    let t = tokens.clone();
    reg.add(
        ToolSchema::new(
            "git.auth_status",
            "Check authentication status for a provider (or all providers).",
        )
        .opt_str_null(
            "provider",
            "Provider to check: github or gitlab; omit to report all providers.",
        )
        .opt_str_null(
            "host",
            "Host to check; omit to report every host the caller holds a token for.",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let tokens = resolve_tokens(&ctx, t).await?;
                auth_status(
                    &ctx,
                    a.opt_str("provider").as_deref(),
                    a.opt_str("host").as_deref(),
                    &tokens,
                )
            }
        }),
    );

    let t = tokens.clone();
    reg.add(
        ToolSchema::new("git.auth_revoke", "Revoke the stored token for a provider.")
            .opt_str_null("provider", "Provider whose stored token is revoked: github or gitlab.")
            .opt_str_null("host", "Host whose stored token is revoked.")
            .destructive(true)
            .read_only(false)
            .idempotent(true)
            .open_world(false),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let provider = a.opt_str("provider");
                let host = a.opt_str("host");
                let tokens = resolve_tokens(&ctx, t).await?;
                auth_revoke(&ctx, provider.as_deref(), host.as_deref(), &tokens).await
            }
        }),
    );

    let t = tokens;
    reg.add(
        ToolSchema::new(
            "git.token_set",
            "Seed a personal access token you already hold for a host declared in git.hosts, \
             without the interactive device flow. The token is never echoed back.",
        )
        .req_str("host", "Hostname declared in git.hosts to store the token for.")
        .req_str(
            "token",
            "The personal access token value. Never echoed back; 1 to 8192 characters.",
        )
        .opt_str_null(
            "expires_at",
            "RFC 3339 timestamp the token expires at; omit or null for a token that never \
             expires.",
        )
        .destructive(false)
        .read_only(false)
        .idempotent(true)
        .open_world(false),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let host = a.str("host")?;
                let token = a.str("token")?;
                let expires_at = match a.raw("expires_at") {
                    None => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(_) => {
                        return Err(ToolError::invalid_argument(
                            "argument 'expires_at' must be an RFC 3339 timestamp string",
                        ));
                    }
                };
                let tokens = resolve_tokens(&ctx, t).await?;
                token_set(&ctx, &host, &token, expires_at, &tokens).await
            }
        }),
    );
}

async fn resolve_tokens(
    ctx: &ToolCtx,
    injected: Option<Arc<OAuthTokenStore>>,
) -> Result<Arc<OAuthTokenStore>> {
    match injected {
        Some(t) => Ok(t),
        None => token_store(&ctx.state.config, ctx.state.stores.relational()).await,
    }
}

/// The caller identity. Empty means the request reached a tool without passing
/// identity verification, which is a bug, not a permission problem.
fn require_identity(ctx: &ToolCtx) -> Result<String> {
    if ctx.person.trim().is_empty() {
        return Err(ToolError::unauthenticated("no authenticated identity in context"));
    }
    Ok(ctx.person.clone())
}

// ── implementations ─────────────────────────────────────────────────────────

/// The canonical public host for a provider, used when `git.auth` receives
/// neither `host` nor `instance_url` (DEC-018).
fn canonical_host(provider: &str) -> Option<&'static str> {
    match provider {
        "github" => Some("github.com"),
        "gitlab" => Some("gitlab.com"),
        _ => None,
    }
}

fn provider_name(p: crate::git::remote::Provider) -> &'static str {
    use crate::git::remote::Provider;
    match p {
        Provider::Github => "github",
        Provider::Gitlab => "gitlab",
        Provider::Generic => "generic",
        Provider::Anonymous => "anonymous",
    }
}

/// Parse the hostname out of a self-hosted instance URL, lowercased.
fn hostname_of(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url).map_err(|e| {
        ToolError::invalid_argument(format!("instance_url '{url}' is not a valid URL: {e}"))
    })?;
    let host = parsed.host_str().filter(|h| !h.is_empty()).ok_or_else(|| {
        ToolError::invalid_argument(format!("instance_url '{url}' has no hostname"))
    })?;
    Ok(host.to_ascii_lowercase())
}

/// Resolve, validate and return the host `git.auth` authorizes against
/// (FR-MOD-002, FR-NEW-066). `provider` is already known to be exactly
/// `"github"` or `"gitlab"`. Reuses `crate::git::remote::resolve_host`, the
/// sole reader of `git.hosts`, rather than reimplementing host lookup.
fn resolve_auth_host(
    provider: &str,
    host: Option<&str>,
    instance_url: Option<&str>,
) -> Result<String> {
    let instance_host = instance_url.map(hostname_of).transpose()?;
    let resolved = match (host, &instance_host) {
        (Some(h), Some(ih)) => {
            let h_lower = h.to_ascii_lowercase();
            if &h_lower != ih {
                return Err(ToolError::invalid_argument(format!(
                    "host '{h}' and instance_url hostname '{ih}' disagree; supply matching \
                     values or omit one of them"
                )));
            }
            h_lower
        }
        (Some(h), None) => h.to_ascii_lowercase(),
        (None, Some(ih)) => ih.clone(),
        (None, None) => canonical_host(provider)
            .expect("provider already validated as github or gitlab")
            .to_string(),
    };

    use crate::git::remote::Provider;
    let mapped = crate::git::remote::resolve_host(&resolved).map_err(|_| {
        ToolError::invalid_argument(format!(
            "host '{resolved}' is not declared in git.hosts; declare it under git.hosts \
             before authorizing against it"
        ))
    })?;
    match mapped {
        Provider::Generic | Provider::Anonymous => Err(ToolError::invalid_argument(format!(
            "host '{resolved}' is declared '{}' in git.hosts; the device flow exists only \
             for github and gitlab",
            provider_name(mapped)
        ))),
        Provider::Github | Provider::Gitlab if provider_name(mapped) != provider => {
            Err(ToolError::invalid_argument(format!(
                "host '{resolved}' maps to provider '{}' in git.hosts, not '{provider}': both \
                 must agree",
                provider_name(mapped)
            )))
        }
        _ => Ok(resolved),
    }
}

async fn auth(
    ctx: &ToolCtx,
    provider: &str,
    host: Option<String>,
    instance_url: Option<String>,
    tokens: Arc<OAuthTokenStore>,
    flow: Arc<dyn DeviceFlowClient>,
) -> Result<Value> {
    // Exact match, like the C#: "GitHub" is a client bug worth reporting.
    if provider != "github" && provider != "gitlab" {
        return Err(ToolError::invalid_argument("provider must be 'github' or 'gitlab'"));
    }
    let person = require_identity(ctx)?;
    let resolved_host = resolve_auth_host(provider, host.as_deref(), instance_url.as_deref())?;
    let code = flow.request_device_code(provider, instance_url.as_deref()).await?;

    let message = format!("Open {} and enter code {}", code.verification_uri, code.user_code);
    let result = json!({
        "status": "pending",
        "provider": provider,
        "user_code": code.user_code,
        "verification_uri": code.verification_uri,
        "expires_in": code.expires_in,
        "message": message,
    });

    spawn_poller(flow, tokens, person, resolved_host, provider.to_string(), instance_url, code);
    Ok(result)
}

/// Poll the token endpoint until the user authorizes, refuses, or the code dies.
/// Detached on purpose: `git.auth` must answer immediately so the caller can show
/// the code, and the client then polls `git.auth_status`.
fn spawn_poller(
    flow: Arc<dyn DeviceFlowClient>,
    tokens: Arc<OAuthTokenStore>,
    person: String,
    host: String,
    provider: String,
    instance_url: Option<String>,
    code: DeviceCode,
) {
    let interval = Duration::from_secs(code.interval.max(0) as u64).max(MIN_POLL_INTERVAL);
    let lifetime = Duration::from_secs(code.expires_in.max(0) as u64);
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + lifetime;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(interval).await;
            match flow.poll_for_token(&code).await {
                Ok(poll) if poll.success => {
                    let Some(token) = poll.access_token else { return };
                    if let Err(e) = tokens
                        .store_token(
                            &person,
                            &host,
                            &provider,
                            &token,
                            poll.scopes,
                            Some(poll.expires_at),
                            instance_url,
                        )
                        .await
                    {
                        // The message never contains the token itself.
                        tracing::warn!(
                            "git.auth: cannot store the {provider} token for host {host}: {e}"
                        );
                    }
                    return;
                }
                Ok(poll) if poll.is_terminal_error() => return,
                // authorization_pending, slow_down, or a transient transport
                // error: the user may still be typing the code.
                _ => {}
            }
        }
    });
}

/// `git.auth_status` (FR-MOD-003, FR-NEW-051, DRIFT-010): one entry per
/// `(person, host)` the caller actually holds a token for, ordered by host
/// ascending, filtered by the optional `provider`/`host` arguments. Built
/// from [`OAuthTokenStore::list_for_person`], a single filtered scan of the
/// store keyed to this person, never from `list_ids` (every person,
/// unfiltered) followed by per-id lookups: that shape would both leak across
/// people (FR-NEW-040) and do redundant work.
pub(crate) fn auth_status(
    ctx: &ToolCtx,
    provider: Option<&str>,
    host: Option<&str>,
    tokens: &OAuthTokenStore,
) -> Result<Value> {
    let person = require_identity(ctx)?;
    let mut entries = tokens.list_for_person(&person);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let now = Utc::now();
    let statuses: Vec<Value> = entries
        .into_iter()
        .filter(|(_, s)| provider.is_none_or(|p| p == s.provider))
        .filter(|(h, _)| host.is_none_or(|hh| hh.eq_ignore_ascii_case(h)))
        .map(|(h, s)| {
            // FR-NEW-335: the pull request surface needs read and write, and the
            // answer has three states, so `pr_capable` is null when the recorded
            // scope set says nothing rather than false (DEC-911).
            let read = pr_capability(&s.provider, &s.scopes, PrAccess::Read);
            let write = pr_capability(&s.provider, &s.scopes, PrAccess::Write);
            let capable = match (read.as_bool(), write.as_bool()) {
                (Some(r), Some(w)) => Some(r && w),
                _ => None,
            };
            let mut missing: Vec<String> =
                read.missing().iter().chain(write.missing()).cloned().collect();
            missing.sort();
            missing.dedup();
            json!({
                "host": h,
                "provider": s.provider,
                "validity": if s.is_valid_at(now) { "valid" } else { "expired" },
                "expires_at": s.expires_at.map(round_trip_iso),
                "scopes": s.scopes,
                "pr_read": read.as_bool(),
                "pr_write": write.as_bool(),
                "pr_capable": capable,
                "missing_scopes": missing,
            })
        })
        .collect();
    Ok(json!({"statuses": statuses}))
}

/// `git.auth_revoke` (FR-NEW-051 response shape only: the host-aware
/// revocation semantics belong to US-007, per the story's scope boundary).
/// `host` takes priority when given; otherwise `provider` maps to its
/// canonical public host, the same default `git.auth` uses, so a round trip
/// through the unqualified pre-existing calling convention still finds what
/// it stored. `provider` in the response is resolved from `git.hosts` for
/// the given host, `null` when the host is undeclared (E2E-NEW-190).
pub(crate) async fn auth_revoke(
    ctx: &ToolCtx,
    provider: Option<&str>,
    host: Option<&str>,
    tokens: &OAuthTokenStore,
) -> Result<Value> {
    let person = require_identity(ctx)?;
    let resolved_host = match (host, provider) {
        (Some(h), _) => h.to_string(),
        (None, Some(p)) => canonical_host(p).map(str::to_string).unwrap_or_else(|| p.to_string()),
        (None, None) => {
            return Err(ToolError::invalid_argument(
                "git.auth_revoke requires 'host' or 'provider'",
            ));
        }
    };

    let resolved_provider =
        crate::git::remote::resolve_host(&resolved_host).ok().map(provider_name);

    // A declared host whose git.hosts value disagrees with a supplied
    // provider is rejected, mirroring EXC-003b (FR-NEW-068). An undeclared
    // host has no git.hosts value to disagree with, so it is left alone here:
    // it must stay revocable (FR-NEW-068's undeclared-host business rule).
    if let (Some(p), Some(rp)) = (provider, resolved_provider)
        && p != rp
    {
        return Err(ToolError::invalid_argument(format!(
            "host '{resolved_host}' maps to provider '{rp}' in git.hosts, not '{p}': both must \
             agree"
        )));
    }

    let existed = tokens.get_token(&person, &resolved_host).is_some();
    tokens.revoke_token(&person, &resolved_host).await?;
    Ok(json!({"host": resolved_host, "provider": resolved_provider, "revoked": existed}))
}

/// `git.token_set` (FR-NEW-013): store a token the caller already holds,
/// instead of forcing the interactive device flow for a credential the caller
/// already possesses. The provider is resolved from `git.hosts`
/// (`crate::git::remote::resolve_host`), never supplied by the caller.
pub(crate) async fn token_set(
    ctx: &ToolCtx,
    host: &str,
    token: &str,
    expires_at: Option<String>,
    tokens: &OAuthTokenStore,
) -> Result<Value> {
    let person = require_identity(ctx)?;

    use crate::git::remote::Provider;
    let provider = crate::git::remote::resolve_host(host).map_err(|_| {
        ToolError::invalid_argument(format!(
            "host '{host}' is not declared in git.hosts; declare it under git.hosts before \
             seeding a token for it"
        ))
    })?;
    if provider == Provider::Anonymous {
        return Err(ToolError::invalid_argument(format!(
            "host '{host}' is declared anonymous: an anonymous host holds no credential"
        )));
    }
    let provider_name = match provider {
        Provider::Github => "github",
        Provider::Gitlab => "gitlab",
        Provider::Generic => "generic",
        Provider::Anonymous => unreachable!("handled above"),
    };

    if token.trim().is_empty() {
        return Err(ToolError::invalid_argument("token must not be empty or whitespace-only"));
    }
    if token.chars().count() > MAX_TOKEN_LEN {
        return Err(ToolError::invalid_argument(format!(
            "token must be at most {MAX_TOKEN_LEN} characters"
        )));
    }

    // A past expiry is accepted (DEC-017, FR-NEW-052): the token is simply
    // already expired at seed time, reported so by `git.auth_status` rather
    // than rejected here.
    let expires_at = match expires_at {
        Some(s) => Some(
            DateTime::parse_from_rfc3339(&s)
                .map_err(|_| {
                    ToolError::invalid_argument(format!(
                        "argument 'expires_at' must be an RFC 3339 timestamp, got '{s}'"
                    ))
                })?
                .with_timezone(&Utc),
        ),
        None => None,
    };

    tokens.store_token(&person, host, provider_name, token, Vec::new(), expires_at, None).await?;

    let persistent = tokens.is_persistent();
    let mut out = json!({
        "host": host,
        "provider": provider_name,
        "stored": true,
        "persistent": persistent,
    });
    if !persistent {
        out["message"] = json!(
            "Stored in memory only: this token lives only for the current process and will \
             not survive a restart because MCPFS_TOKEN_KEY is not configured."
        );
    }
    Ok(out)
}

/// The C# `DateTimeOffset.ToString("O")`: seven fractional digits (100ns ticks)
/// plus an explicit offset, which is always UTC here.
fn round_trip_iso(dt: DateTime<Utc>) -> String {
    format!("{}{:07}+00:00", dt.format("%Y-%m-%dT%H:%M:%S."), dt.timestamp_subsec_nanos() / 100)
}

#[cfg(test)]
mod tests {
    use super::super::admin::test_support::Fixture;
    use super::*;
    use crate::errors::code;
    use crate::git::oauth::device_flow::TokenPoll;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PERSON: &str = "dev@test.com";

    /// The C# `IDeviceFlowClient` test double: a canned device code plus a script
    /// of poll answers, the last one repeating.
    struct FakeFlow {
        code: DeviceCode,
        script: Mutex<Vec<TokenPoll>>,
        polls: AtomicUsize,
        requests: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeFlow {
        fn new(script: Vec<TokenPoll>) -> Arc<Self> {
            Arc::new(Self {
                code: DeviceCode {
                    provider: "github".into(),
                    device_code: "device-secret".into(),
                    user_code: "WXYZ-9876".into(),
                    verification_uri: "https://github.com/login/device".into(),
                    // 5 seconds is plenty: the interval floor is 10 ms in tests.
                    expires_in: 5,
                    interval: 0,
                    instance_url: None,
                },
                script: Mutex::new(script),
                polls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
            })
        }

        fn polls(&self) -> usize {
            self.polls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DeviceFlowClient for FakeFlow {
        async fn request_device_code(
            &self,
            provider: &str,
            instance_url: Option<&str>,
        ) -> Result<DeviceCode> {
            self.requests
                .lock()
                .unwrap()
                .push((provider.to_string(), instance_url.map(str::to_string)));
            Ok(DeviceCode { provider: provider.to_string(), ..self.code.clone() })
        }

        async fn poll_for_token(&self, _code: &DeviceCode) -> Result<TokenPoll> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            let mut q = self.script.lock().unwrap();
            Ok(if q.len() > 1 { q.remove(0) } else { q[0].clone() })
        }
    }

    fn future() -> DateTime<Utc> {
        Utc::now() + chrono::Duration::hours(1)
    }

    /// Fixture plus an isolated token store and a fake flow.
    async fn setup(
        script: Vec<TokenPoll>,
    ) -> (Fixture, ToolRegistry, Arc<OAuthTokenStore>, Arc<FakeFlow>) {
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        let tokens = Arc::new(OAuthTokenStore::new());
        let flow = FakeFlow::new(script);
        let mut r = ToolRegistry::new();
        register_with(&mut r, Some(tokens.clone()), Some(flow.clone()));
        (f, r, tokens, flow)
    }

    /// Wait (bounded) for a condition the background poller drives.
    async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn every_git_auth_tool_is_registered() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        assert_eq!(r.len(), 4);
        for name in ["git.auth", "git.auth_status", "git.auth_revoke", "git.token_set"] {
            assert!(r.resolve(name).is_some(), "{name} is missing");
        }
    }

    #[tokio::test]
    async fn git_auth_schema_matches_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.auth").unwrap().schema;
        assert_eq!(
            s.description,
            "Start OAuth device flow for GitHub or GitLab. Returns user_code and verification_uri."
        );
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "provider":{"description":"OAuth provider: github or gitlab.","type":"string"},
                 "host":{"description":"Host declared in git.hosts to authorize against; omit to use the provider's canonical public host (github.com or gitlab.com).","type":"string","default":null},
                 "instance_url":{"description":"Optional self-hosted instance URL (e.g. GitLab Enterprise).","type":"string","default":null}},
               "required":["provider"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    #[tokio::test]
    async fn git_auth_status_schema_has_no_required_parameter() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.auth_status").unwrap().schema;
        assert_eq!(s.description, "Check authentication status for a provider (or all providers).");
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "provider":{"description":"Provider to check: github or gitlab; omit to report all providers.","type":"string","default":null},
                 "host":{"description":"Host to check; omit to report every host the caller holds a token for.","type":"string","default":null}}}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);

        let rev = &r.resolve("git.auth_revoke").unwrap().schema;
        assert_eq!(rev.description, "Revoke the stored token for a provider.");
        assert!(
            rev.input_schema().get("required").is_none(),
            "no required parameter on auth_revoke: host and provider are both optional"
        );
    }

    #[test]
    fn git_auth_returns_pending_immediately() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, _tokens, flow) =
                setup(vec![TokenPoll::pending("authorization_pending")]).await;
            let out = f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();
            assert_eq!(out["status"], "pending");
            assert_eq!(out["provider"], "github");
            assert_eq!(out["user_code"], "WXYZ-9876");
            assert_eq!(out["verification_uri"], "https://github.com/login/device");
            assert_eq!(out["expires_in"], 5);
            assert_eq!(
                out["message"],
                "Open https://github.com/login/device and enter code WXYZ-9876"
            );
            assert_eq!(flow.requests.lock().unwrap()[0], ("github".to_string(), None));
        });
    }

    #[test]
    fn pending_then_success_stores_the_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, flow) = setup(vec![
                TokenPoll::pending("authorization_pending"),
                TokenPoll::granted("gho_stored", vec!["repo".into()], future()),
            ])
            .await;
            f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();

            assert!(
                eventually(|| tokens.has_valid_token(PERSON, "github.com")).await,
                "the poller must store the token, keyed by the canonical default host"
            );
            assert!(flow.polls() >= 2, "the pending answer must not end the loop");
            let s = tokens.get_token(PERSON, "github.com").unwrap();
            assert_eq!(s.access_token, "gho_stored");
            assert_eq!(s.scopes, vec!["repo"]);
            assert!(s.instance_url.is_none());
            assert!(
                tokens.get_token(PERSON, "github").is_none(),
                "nothing stored under the bare provider name anymore"
            );
        });
    }

    #[test]
    fn a_refused_authorization_stores_nothing_and_stops_polling() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, flow) = setup(vec![TokenPoll::pending("access_denied")]).await;
            f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();

            assert!(eventually(|| flow.polls() >= 1).await, "the poller must run at least once");
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(tokens.get_token(PERSON, "github.com").is_none(), "no token on refusal");
            let after = flow.polls();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(flow.polls(), after, "access_denied is terminal");
        });
    }

    #[test]
    fn gitlab_keeps_the_instance_url_on_the_stored_session() {
        with_git_hosts_lock(async {
            declare_hosts(&[("gitlab.example.test", "gitlab")]);
            let (f, r, tokens, flow) =
                setup(vec![TokenPoll::granted("glpat", vec!["api".into()], future())]).await;
            f.call(
                &r,
                PERSON,
                "git.auth",
                json!({"provider":"gitlab","instance_url":"https://gitlab.example.test"}),
            )
            .await
            .unwrap();

            assert!(eventually(|| tokens.has_valid_token(PERSON, "gitlab.example.test")).await);
            let s = tokens.get_token(PERSON, "gitlab.example.test").unwrap();
            assert_eq!(s.instance_url.as_deref(), Some("https://gitlab.example.test"));
            assert!(
                tokens.get_token(PERSON, "gitlab.com").is_none(),
                "nothing stored under the canonical default host"
            );
            assert_eq!(
                flow.requests.lock().unwrap()[0],
                ("gitlab".to_string(), Some("https://gitlab.example.test".to_string()))
            );
        });
    }

    /// E2E-NEW-049: preserves the pre-existing exact-match rejection
    /// (`crates/mcp-fs/src/tools/git_auth.rs:160-162` before this story).
    #[tokio::test]
    async fn an_unknown_provider_is_rejected_before_any_http_call() {
        let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
        for bad in ["bitbucket", "GitHub", ""] {
            let e = f.call(&r, PERSON, "git.auth", json!({"provider":bad})).await.unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT, "'{bad}' must be rejected");
            assert_eq!(e.message, "provider must be 'github' or 'gitlab'");
        }
        assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
    }

    /// E2E-NEW-046: a host mapped to a different provider is rejected, naming
    /// both, before any device code is requested.
    #[test]
    fn e2e_new_046_a_host_mapped_to_a_different_provider_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
            let e = f
                .call(&r, PERSON, "git.auth", json!({"provider":"gitlab","host":"github.ibm.com"}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("gitlab"), "{}", e.message);
            assert!(e.message.contains("github"), "{}", e.message);
            assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
        });
    }

    /// E2E-NEW-047: a generic host is rejected, stating the device flow exists
    /// only for github and gitlab.
    #[test]
    fn e2e_new_047_a_generic_host_is_rejected_for_the_device_flow() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.auth",
                    json!({"provider":"github","host":"git.acme.internal"}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("github") && e.message.contains("gitlab"), "{}", e.message);
            assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
        });
    }

    /// E2E-NEW-048: an anonymous host is rejected for the same reason.
    #[test]
    fn e2e_new_048_an_anonymous_host_is_rejected_for_the_device_flow() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.auth",
                    json!({"provider":"github","host":"public.example.org"}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("github") && e.message.contains("gitlab"), "{}", e.message);
            assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
        });
    }

    /// E2E-NEW-050: an undeclared host is rejected for the device flow, naming
    /// it, before any device code is requested.
    #[test]
    fn e2e_new_050_an_undeclared_host_is_rejected_for_the_device_flow() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.auth",
                    json!({"provider":"github","host":"git.unknown.test"}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("git.unknown.test"), "{}", e.message);
            assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
        });
    }

    /// E2E-NEW-044: the device flow stores under the named host, and nothing
    /// under the canonical default.
    #[test]
    fn e2e_new_044_device_flow_stores_under_the_named_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) =
                setup(vec![TokenPoll::granted("ghp_ent", vec!["repo".into()], future())]).await;
            f.call(&r, PERSON, "git.auth", json!({"provider":"github","host":"github.ibm.com"}))
                .await
                .unwrap();

            assert!(eventually(|| tokens.has_valid_token(PERSON, "github.ibm.com")).await);
            assert!(
                tokens.get_token(PERSON, "github.com").is_none(),
                "nothing stored for the canonical default host"
            );
        });
    }

    /// E2E-NEW-045: an omitted host defaults to the canonical public host, per
    /// provider.
    #[test]
    fn e2e_new_045_an_omitted_host_defaults_to_the_canonical_public_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) =
                setup(vec![TokenPoll::granted("ghp_pub", vec![], future())]).await;
            f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();
            assert!(eventually(|| tokens.has_valid_token(PERSON, "github.com")).await);

            let (f2, r2, tokens2, _flow2) =
                setup(vec![TokenPoll::granted("glpat_pub", vec![], future())]).await;
            f2.call(&r2, PERSON, "git.auth", json!({"provider":"gitlab"})).await.unwrap();
            assert!(eventually(|| tokens2.has_valid_token(PERSON, "gitlab.com")).await);
        });
    }

    /// E2E-NEW-227: `instance_url` supplies the host when `host` is omitted.
    #[test]
    fn e2e_new_227_instance_url_supplies_the_host_when_host_is_omitted() {
        with_git_hosts_lock(async {
            declare_hosts(&[("gitlab.acme.corp", "gitlab")]);
            let (f, r, tokens, flow) =
                setup(vec![TokenPoll::granted("glpat_ent", vec![], future())]).await;
            f.call(
                &r,
                PERSON,
                "git.auth",
                json!({"provider":"gitlab","instance_url":"https://gitlab.acme.corp"}),
            )
            .await
            .unwrap();

            assert!(eventually(|| tokens.has_valid_token(PERSON, "gitlab.acme.corp")).await);
            assert!(tokens.get_token(PERSON, "gitlab.com").is_none());
            assert_eq!(
                tokens.get_token(PERSON, "gitlab.acme.corp").unwrap().instance_url.as_deref(),
                Some("https://gitlab.acme.corp")
            );
            assert_eq!(
                flow.requests.lock().unwrap()[0],
                ("gitlab".to_string(), Some("https://gitlab.acme.corp".to_string()))
            );
        });
    }

    /// E2E-NEW-228: a disagreeing `instance_url` and `host` are rejected,
    /// naming both, before any device code is requested.
    #[test]
    fn e2e_new_228_a_disagreeing_instance_url_and_host_are_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(&[("gitlab.acme.corp", "gitlab")]);
            let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.auth",
                    json!({
                        "provider":"gitlab",
                        "instance_url":"https://gitlab.acme.corp",
                        "host":"gitlab.com",
                    }),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("gitlab.acme.corp"), "{}", e.message);
            assert!(e.message.contains("gitlab.com"), "{}", e.message);
            assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
        });
    }

    /// E2E-NEW-229: the canonical default applies only when both `host` and
    /// `instance_url` are absent.
    #[test]
    fn e2e_new_229_the_canonical_default_applies_only_when_both_are_absent() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) =
                setup(vec![TokenPoll::granted("glpat_def", vec![], future())]).await;
            f.call(&r, PERSON, "git.auth", json!({"provider":"gitlab"})).await.unwrap();
            assert!(eventually(|| tokens.has_valid_token(PERSON, "gitlab.com")).await);
            assert!(tokens.get_token(PERSON, "gitlab.com").unwrap().instance_url.is_none());
        });
    }

    #[tokio::test]
    async fn auth_status_reports_one_provider() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let out =
            f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
        assert_eq!(out, json!({"statuses": []}), "a host with no token is never listed");

        tokens
            .store_token(
                PERSON,
                "github.com",
                "github",
                "tok",
                vec!["repo".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();
        let out =
            f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0]["host"], "github.com");
        assert_eq!(statuses[0]["validity"], "valid");
        assert_eq!(statuses[0]["scopes"], json!(["repo"]));
        let expires = statuses[0]["expires_at"].as_str().unwrap();
        assert!(expires.ends_with("+00:00"), "got {expires}");
        assert!(!serde_json::to_string(&out).unwrap().contains("tok"));
    }

    #[tokio::test]
    async fn auth_status_reports_all_providers_when_none_is_given() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "gitlab.com",
                "gitlab",
                "tok",
                vec!["api".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 1, "a host with no token must never be listed (DRIFT-010)");
        assert_eq!(statuses[0]["host"], "gitlab.com");
        assert_eq!(statuses[0]["provider"], "gitlab");
        assert_eq!(statuses[0]["validity"], "valid");
        assert_eq!(statuses[0]["scopes"], json!(["api"]));
    }

    /// E2E-MOD-002: an expired token reports `validity: "expired"`, distinct
    /// from absent, and a host with no token is never listed at all
    /// (DRIFT-010's proving test: before the fix, `github.com` would have
    /// been the only reachable entry through a fixed provider list and
    /// `gitlab.acme.corp` had no way to be represented as "not held" versus
    /// "held but expired").
    #[tokio::test]
    async fn an_expired_token_reports_as_unauthenticated() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "github.ibm.com",
                "github",
                "stale",
                vec![],
                Some(Utc::now() - chrono::Duration::minutes(1)),
                None,
            )
            .await
            .unwrap();
        // nothing stored for gitlab.acme.corp

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 1, "only the host actually holding a token is listed");
        assert_eq!(statuses[0]["host"], "github.ibm.com");
        assert_eq!(statuses[0]["validity"], "expired");
        assert!(!statuses.iter().any(|s| s["host"] == "gitlab.acme.corp"));
    }

    #[test]
    fn revoke_clears_the_token_and_is_idempotent() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            tokens
                .store_token(PERSON, "github.com", "github", "tok", vec![], Some(future()), None)
                .await
                .unwrap();

            let out =
                f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
            assert_eq!(out, json!({"host":"github.com","provider":"github","revoked":true}));
            assert!(tokens.get_token(PERSON, "github.com").is_none());

            // revoking again must not fail, and now truthfully reports nothing was there
            let out2 =
                f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
            assert_eq!(out2["revoked"], false);
        });
    }

    #[tokio::test]
    async fn tokens_are_per_person() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(PERSON, "github.com", "github", "mine", vec![], Some(future()), None)
            .await
            .unwrap();
        let out = f
            .call(&r, "other@test.com", "git.auth_status", json!({"provider":"github"}))
            .await
            .unwrap();
        assert_eq!(out, json!({"statuses": []}), "another person must not inherit a token");
    }

    #[tokio::test]
    async fn an_unauthenticated_caller_is_rejected() {
        let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        for (name, args) in [
            ("git.auth", json!({"provider":"github"})),
            ("git.auth_status", json!({})),
            ("git.auth_revoke", json!({"provider":"github"})),
        ] {
            let e = f.call(&r, "  ", name, args).await.unwrap_err();
            assert_eq!(e.code, code::UNAUTHENTICATED, "{name}");
        }
    }

    // ── FR-NEW-051: fixed response shapes for auth_status and auth_revoke ────

    /// E2E-NEW-063: status reports one entry per host.
    #[tokio::test]
    async fn e2e_new_063_status_reports_one_entry_per_host() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "github.com",
                "github",
                "tok1",
                vec!["repo".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();
        tokens
            .store_token(
                PERSON,
                "github.ibm.com",
                "github",
                "tok2",
                vec!["api".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 2);
        for s in statuses {
            assert_eq!(s["provider"], "github");
            assert!(s["host"].is_string());
            assert!(s["validity"].is_string());
            assert!(s.get("expires_at").is_some());
        }
    }

    /// E2E-NEW-064: status filters by provider and by host.
    #[tokio::test]
    async fn e2e_new_064_status_filters_by_provider_and_by_host() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(PERSON, "github.com", "github", "t1", vec![], Some(future()), None)
            .await
            .unwrap();
        tokens
            .store_token(PERSON, "github.ibm.com", "github", "t2", vec![], Some(future()), None)
            .await
            .unwrap();
        tokens
            .store_token(PERSON, "gitlab.acme.corp", "gitlab", "t3", vec![], Some(future()), None)
            .await
            .unwrap();

        let by_provider =
            f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
        assert_eq!(by_provider["statuses"].as_array().unwrap().len(), 2);

        let by_host =
            f.call(&r, PERSON, "git.auth_status", json!({"host":"github.ibm.com"})).await.unwrap();
        let host_statuses = by_host["statuses"].as_array().unwrap();
        assert_eq!(host_statuses.len(), 1);
        assert_eq!(host_statuses[0]["host"], "github.ibm.com");
    }

    /// E2E-NEW-065: status never includes a token value.
    #[tokio::test]
    async fn e2e_new_065_status_never_includes_a_token_value() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let token = "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH";
        tokens
            .store_token(
                PERSON,
                "github.com",
                "github",
                token,
                vec!["repo".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let serialized = serde_json::to_string(&out).unwrap();
        for start in 0..=(token.len() - 8) {
            let chunk = &token[start..start + 8];
            assert!(!serialized.contains(chunk), "response must not contain '{chunk}'");
        }
    }

    /// E2E-NEW-069: status with no tokens returns an empty list, not an error.
    #[tokio::test]
    async fn e2e_new_069_status_with_no_tokens_returns_an_empty_list() {
        let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        assert_eq!(out, json!({"statuses": []}));
    }

    /// E2E-NEW-070: status distinguishes expired from absent.
    #[tokio::test]
    async fn e2e_new_070_status_distinguishes_expired_from_absent() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "github.ibm.com",
                "github",
                "stale",
                vec![],
                Some(Utc::now() - chrono::Duration::minutes(1)),
                None,
            )
            .await
            .unwrap();

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0]["host"], "github.ibm.com");
        assert_eq!(statuses[0]["validity"], "expired");
        assert!(!statuses.iter().any(|s| s["host"] == "gitlab.acme.corp"));
    }

    /// E2E-NEW-161: existing callers keep working without `host`, defaulting
    /// to the canonical public host end to end across all three tools.
    #[test]
    fn e2e_new_161_existing_callers_keep_working_without_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) =
                setup(vec![TokenPoll::granted("gho_x", vec!["repo".into()], future())]).await;

            f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();
            assert!(eventually(|| tokens.has_valid_token(PERSON, "github.com")).await);

            let status =
                f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
            assert_eq!(status["statuses"].as_array().unwrap().len(), 1);

            let revoke =
                f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
            assert_eq!(revoke["revoked"], true);
            assert!(tokens.get_token(PERSON, "github.com").is_none());
        });
    }

    /// E2E-NEW-188: `git.auth_status` returns the declared shape, ordered by
    /// host ascending, with no `authenticated` key anywhere.
    #[tokio::test]
    async fn e2e_new_188_auth_status_returns_the_declared_shape() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "github.com",
                "github",
                "t1",
                vec!["repo".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();
        tokens
            .store_token(
                PERSON,
                "github.ibm.com",
                "github",
                "t2",
                vec![],
                Some(Utc::now() - chrono::Duration::minutes(1)),
                None,
            )
            .await
            .unwrap();

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0]["host"], "github.com");
        assert_eq!(statuses[0]["validity"], "valid");
        assert_eq!(statuses[1]["host"], "github.ibm.com");
        assert_eq!(statuses[1]["validity"], "expired");
        let serialized = serde_json::to_string(&out).unwrap();
        assert!(!serialized.contains("authenticated"), "no authenticated key anywhere");
    }

    /// E2E-NEW-189: `expires_at` is null for a non-expiring token.
    #[tokio::test]
    async fn e2e_new_189_expires_at_is_null_for_a_non_expiring_token() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens.store_token(PERSON, "github.com", "github", "t1", vec![], None, None).await.unwrap();
        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0]["expires_at"], Value::Null);
        assert_eq!(statuses[0]["validity"], "valid");
    }

    /// E2E-NEW-190: revoking an absent host returns the declared shape.
    #[tokio::test]
    async fn e2e_new_190_revoking_an_absent_host_returns_the_declared_shape() {
        let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let out = f
            .call(&r, PERSON, "git.auth_revoke", json!({"host":"git.unknown.test"}))
            .await
            .unwrap();
        assert_eq!(out, json!({"host":"git.unknown.test","provider":null,"revoked":false}));
    }

    // ── US-007: git.auth_revoke removes exactly one host (FR-MOD-004, FR-NEW-068) ──

    /// E2E-NEW-066: revoke removes exactly one host, the other is untouched.
    #[test]
    fn e2e_new_066_revoke_removes_exactly_one_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            tokens
                .store_token(PERSON, "github.com", "github", "t1", vec![], Some(future()), None)
                .await
                .unwrap();
            tokens
                .store_token(PERSON, "github.ibm.com", "github", "t2", vec![], Some(future()), None)
                .await
                .unwrap();

            let out = f
                .call(
                    &r,
                    PERSON,
                    "git.auth_revoke",
                    json!({"provider":"github","host":"github.ibm.com"}),
                )
                .await
                .unwrap();
            assert_eq!(out, json!({"host":"github.ibm.com","provider":"github","revoked":true}));
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
            assert!(
                tokens.get_token(PERSON, "github.com").is_some(),
                "github.com must be untouched"
            );
        });
    }

    /// E2E-NEW-067: revoke with an omitted host targets only the canonical host.
    #[test]
    fn e2e_new_067_revoke_with_omitted_host_targets_only_the_canonical_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            tokens
                .store_token(PERSON, "github.com", "github", "t1", vec![], Some(future()), None)
                .await
                .unwrap();
            tokens
                .store_token(PERSON, "github.ibm.com", "github", "t2", vec![], Some(future()), None)
                .await
                .unwrap();

            let out =
                f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
            assert_eq!(out["host"], "github.com");
            assert!(tokens.get_token(PERSON, "github.com").is_none());
            assert!(
                tokens.get_token(PERSON, "github.ibm.com").is_some(),
                "revocation must never cross hosts"
            );
        });
    }

    /// E2E-NEW-068: revoking an absent host is idempotent.
    #[test]
    fn e2e_new_068_revoking_an_absent_host_is_idempotent() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;

            let out1 = f
                .call(&r, PERSON, "git.auth_revoke", json!({"host":"github.ibm.com"}))
                .await
                .unwrap();
            assert_eq!(out1["revoked"], false);

            let out2 = f
                .call(&r, PERSON, "git.auth_revoke", json!({"host":"github.ibm.com"}))
                .await
                .unwrap();
            assert_eq!(out2["revoked"], false);
        });
    }

    /// E2E-NEW-233: revoke by provider alone targets the canonical host.
    #[test]
    fn e2e_new_233_revoke_by_provider_alone_targets_the_canonical_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            tokens
                .store_token(PERSON, "github.com", "github", "t1", vec![], Some(future()), None)
                .await
                .unwrap();
            tokens
                .store_token(PERSON, "github.ibm.com", "github", "t2", vec![], Some(future()), None)
                .await
                .unwrap();

            let out =
                f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
            assert_eq!(out, json!({"host":"github.com","provider":"github","revoked":true}));
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_some());
        });
    }

    /// E2E-NEW-234: revoke by host alone succeeds without a provider.
    #[test]
    fn e2e_new_234_revoke_by_host_alone_succeeds_without_a_provider() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            tokens
                .store_token(PERSON, "github.ibm.com", "github", "t2", vec![], Some(future()), None)
                .await
                .unwrap();

            let out = f
                .call(&r, PERSON, "git.auth_revoke", json!({"host":"github.ibm.com"}))
                .await
                .unwrap();
            assert_eq!(out, json!({"host":"github.ibm.com","provider":"github","revoked":true}));
        });
    }

    /// E2E-NEW-235: revoke with neither parameter is rejected, naming both.
    #[tokio::test]
    async fn e2e_new_235_revoke_with_neither_parameter_is_rejected() {
        let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let e = f.call(&r, PERSON, "git.auth_revoke", json!({})).await.unwrap_err();
        assert_eq!(e.code, code::INVALID_ARGUMENT);
        assert!(e.message.contains("provider"), "{}", e.message);
        assert!(e.message.contains("host"), "{}", e.message);
    }

    /// E2E-NEW-236: revoke with a disagreeing provider and host is rejected.
    #[test]
    fn e2e_new_236_revoke_with_disagreeing_provider_and_host_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            tokens
                .store_token(PERSON, "github.ibm.com", "github", "t2", vec![], Some(future()), None)
                .await
                .unwrap();

            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.auth_revoke",
                    json!({"provider":"gitlab","host":"github.ibm.com"}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("gitlab"), "{}", e.message);
            assert!(e.message.contains("github.ibm.com"), "{}", e.message);
            assert!(
                tokens.get_token(PERSON, "github.ibm.com").is_some(),
                "a rejected call must not revoke anything"
            );
        });
    }

    /// E2E-NEW-237: revoking an undeclared host is possible and reports a null provider.
    #[tokio::test]
    async fn e2e_new_237_revoking_an_undeclared_host_reports_a_null_provider() {
        let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let out = f
            .call(&r, PERSON, "git.auth_revoke", json!({"host":"git.unknown.test"}))
            .await
            .unwrap();
        assert_eq!(out, json!({"host":"git.unknown.test","provider":null,"revoked":false}));
    }

    #[test]
    fn round_trip_iso_has_seven_fractional_digits() {
        let dt = DateTime::from_timestamp(1_700_000_000, 123_456_789).unwrap();
        assert_eq!(round_trip_iso(dt), "2023-11-14T22:13:20.1234567+00:00");
        let whole = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        assert_eq!(round_trip_iso(whole), "2023-11-14T22:13:20.0000000+00:00");
    }

    // ── git.token_set (US-004) ───────────────────────────────────────────────────────
    //
    // Every test touching the process wide `git.hosts` map is a plain `#[test]`
    // run through `with_git_hosts_lock`, mirroring `tools/git.rs`'s own
    // convention for the same reason: the map is a `static`, and holding the
    // lock across an `.await` (needed for the whole setup-through-assertions
    // span) is what `with_git_hosts_lock`'s private `block_on` achieves without
    // tripping clippy's `await_holding_lock`.
    //
    // E2E-NEW-022 is a store level test: it proves the contract
    // `resolve_clone_credential` (`crates/mcp-fs/src/tools/git.rs`) actually
    // reads, now by the real host (T-CONVERGE-001): the exact token
    // `git.token_set` stores for `(person, host)` comes back unmodified, ready
    // to be supplied to the remote as `oauth2:<token>`
    // (`crates/mcp-fs/src/tools/git.rs:1046-1049`). The tool-layer proof that
    // `git.token_set` and `git.remote_clone` share this exact lookup, both
    // through their real registered tool names, lives in `tools/git.rs`'s
    // `e2e_new_defect_token_set_then_remote_clone_share_the_real_host`.

    const GHP: &str = "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH";

    const REFERENCE_HOSTS: &[(&str, &str)] = &[
        ("github.com", "github"),
        ("github.ibm.com", "github"),
        ("gitlab.com", "gitlab"),
        ("gitlab.acme.corp", "gitlab"),
        ("git.acme.internal", "generic"),
        ("public.example.org", "anonymous"),
    ];

    fn declare_hosts(pairs: &[(&str, &str)]) {
        let mut cfg = crate::config::GitConfig::default();
        cfg.hosts.0 = pairs.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        crate::git::remote::validate_hosts(&cfg).expect("a valid host map");
    }

    fn with_git_hosts_lock<F: std::future::Future>(f: F) -> F::Output {
        let _guard = crate::git::remote::tests::lock_for_test();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    async fn token_set_registry(tokens: Arc<OAuthTokenStore>) -> (Fixture, ToolRegistry) {
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        let mut r = ToolRegistry::new();
        register_with(&mut r, Some(tokens), None);
        (f, r)
    }

    /// A `RelationalDb` wrapping a real in-memory SQLite database: `migrate`
    /// and `query` delegate untouched, so opening the store succeeds
    /// normally, but `execute` fails once [`Self::arm`] is called. `upsert` is
    /// the only caller of `execute` on this trait, so arming it fails exactly
    /// the persistence write `store_token` performs.
    struct FailingDb {
        inner: crate::storage::rel::SqliteRelationalDb,
        armed: std::sync::atomic::AtomicBool,
    }

    impl FailingDb {
        fn new() -> Self {
            Self {
                inner: crate::storage::rel::SqliteRelationalDb::open_in_memory().unwrap(),
                armed: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn arm(&self) {
            self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl crate::storage::rel::RelationalDb for FailingDb {
        fn dialect(&self) -> crate::storage::rel::Dialect {
            self.inner.dialect()
        }

        async fn execute(&self, query: &crate::storage::rel::Query) -> Result<u64> {
            if self.armed.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ToolError::internal("failingdb: persistence unavailable"));
            }
            self.inner.execute(query).await
        }

        async fn query(
            &self,
            query: &crate::storage::rel::Query,
        ) -> Result<Vec<crate::storage::rel::RowValues>> {
            self.inner.query(query).await
        }

        async fn begin(&self) -> Result<Box<dyn crate::storage::rel::RelationalTx>> {
            self.inner.begin().await
        }

        async fn migrate(&self, schema: &crate::storage::rel::SchemaSet) -> Result<()> {
            self.inner.migrate(schema).await
        }
    }

    async fn failing_persistent_store() -> (Arc<FailingDb>, Arc<OAuthTokenStore>) {
        let db = Arc::new(FailingDb::new());
        let persistence = Arc::new(
            crate::git::oauth::persistence::RelationalOAuthPersistence::open(
                db.clone(),
                [30u8; crate::git::oauth::cipher::KEY_SIZE],
            )
            .await
            .unwrap(),
        );
        let store = Arc::new(OAuthTokenStore::with_persistence(persistence).await.unwrap());
        (db, store)
    }

    #[tokio::test]
    async fn git_token_set_is_registered_and_takes_no_provider_parameter() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.token_set").unwrap().schema;
        let schema = s.input_schema();
        assert!(schema["properties"].get("provider").is_none());
        assert_eq!(schema["required"], json!(["host", "token"]));
    }

    /// E2E-NEW-159/160 / FR-NEW-071: `git.token_set` is a new tool (US-004);
    /// its full `inputSchema` is pinned inline, ahead of the whole-surface
    /// golden comparison, so a drift shows up here with a tool-specific
    /// message too. `host` and `token` required, `expires_at` optional and
    /// nullable.
    #[tokio::test]
    async fn git_token_set_schema_matches_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.token_set").unwrap().schema;
        assert_eq!(
            s.description,
            "Seed a personal access token you already hold for a host declared in git.hosts, \
             without the interactive device flow. The token is never echoed back."
        );
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "host":{"description":"Hostname declared in git.hosts to store the token for.","type":"string"},
                 "token":{"description":"The personal access token value. Never echoed back; 1 to 8192 characters.","type":"string"},
                 "expires_at":{"description":"RFC 3339 timestamp the token expires at; omit or null for a token that never expires.","type":"string","default":null}},
               "required":["host","token"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    /// E2E-NEW-246 / FR-NEW-071: `expires_at` is genuinely optional, not just
    /// declared so: a call omitting it succeeds outright.
    #[test]
    fn e2e_new_246_token_set_without_expires_at_succeeds() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(out["stored"], true);
            assert!(tokens.has_valid_token(PERSON, "github.ibm.com"));
        });
    }

    #[test]
    fn e2e_new_020_seeding_stores_a_token_for_a_declared_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            // Persistence configured (unarmed, so it succeeds), matching
            // FR-NEW-013's example output of `"persistent": true` exactly.
            let (_db, tokens) = failing_persistent_store().await;
            let (f, r) = token_set_registry(tokens.clone()).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(
                out,
                json!({"host":"github.ibm.com","provider":"github","stored":true,"persistent":true})
            );

            let s = tokens.get_token(PERSON, "github.ibm.com").unwrap();
            assert_eq!(s.provider, "github");
            assert_eq!(s.access_token, GHP);
            assert!(tokens.has_valid_token(PERSON, "github.ibm.com"), "validity is 'valid'");
        });
    }

    #[test]
    fn e2e_new_021_seeding_derives_the_provider_from_the_map_not_the_caller() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let schema = r.resolve("git.token_set").unwrap().schema.input_schema();
            assert!(
                schema["properties"].get("provider").is_none(),
                "the tool schema must expose no provider parameter"
            );

            f.call(&r, PERSON, "git.token_set", json!({"host":"git.acme.internal","token":GHP}))
                .await
                .unwrap();
            assert_eq!(tokens.get_token(PERSON, "git.acme.internal").unwrap().provider, "generic");
        });
    }

    #[test]
    fn e2e_new_022_a_seeded_token_is_the_exact_credential_a_clone_would_supply() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(out["auth".to_string()], Value::Null, "no such key: not this tool's shape");
            assert_eq!(out["provider"], "github");

            let s = tokens.get_token(PERSON, "github.ibm.com").unwrap();
            assert_eq!(
                s.access_token, GHP,
                "the exact bytes a clone would pass as the oauth2 password"
            );
            assert!(s.is_valid_at(Utc::now()));
        });
    }

    #[test]
    fn e2e_new_023_the_response_never_echoes_the_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            let serialized = serde_json::to_string(&out).unwrap();
            for start in 0..=(GHP.len() - 8) {
                let chunk = &GHP[start..start + 8];
                assert!(!serialized.contains(chunk), "response must not contain '{chunk}'");
            }
        });
    }

    #[test]
    fn e2e_new_024_an_empty_token_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":""}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_025_a_whitespace_only_token_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.token_set",
                    json!({"host":"github.ibm.com","token":"   \t\n "}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_026_a_token_over_the_bound_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let long = "a".repeat(8193);
            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":long}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_027_an_undeclared_host_is_rejected_for_seeding() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"git.unknown.test","token":GHP}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("git.unknown.test"), "{}", e.message);
            assert!(tokens.get_token(PERSON, "git.unknown.test").is_none());
        });
    }

    #[test]
    fn e2e_new_028_an_anonymous_host_is_rejected_for_seeding() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"public.example.org","token":GHP}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("public.example.org"), "{}", e.message);
            assert!(
                e.message.to_ascii_lowercase().contains("anonymous")
                    && e.message.to_ascii_lowercase().contains("no credential"),
                "the message must state an anonymous host holds no credential: {}",
                e.message
            );
        });
    }

    #[test]
    fn e2e_new_029_an_unauthenticated_caller_is_rejected_for_token_set() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens).await;

            let e = f
                .call(&r, "  ", "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::UNAUTHENTICATED);
        });
    }

    #[test]
    fn e2e_new_030_a_failing_persistence_write_fails_the_call() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            assert!(tokens.is_persistent());
            db.arm();
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap_err();
            assert_ne!(e.code, code::INVALID_ARGUMENT, "distinguishable from a validation failure");
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none(), "never reports stored");
        });
    }

    #[test]
    fn e2e_new_031_seeding_twice_overwrites_leaving_one_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_FIRST0000000000000000000000000"}),
            )
            .await
            .unwrap();
            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_SECOND000000000000000000000000"}),
            )
            .await
            .unwrap();

            assert_eq!(
                tokens.list_ids(),
                vec![(PERSON.to_string(), "github.ibm.com".to_string())],
                "exactly one token stored for (person, host)"
            );
            assert_eq!(
                tokens.get_token(PERSON, "github.ibm.com").unwrap().access_token,
                "ghp_SECOND000000000000000000000000"
            );
        });
    }

    #[test]
    fn e2e_new_034_a_token_at_exactly_the_bound_is_accepted() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let exact = "a".repeat(8192);
            f.call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":exact}))
                .await
                .unwrap();
            let stored = tokens.get_token(PERSON, "github.ibm.com").unwrap().access_token;
            assert_eq!(stored.len(), 8192, "stored intact, not truncated");
        });
    }

    #[test]
    fn e2e_new_035_memory_only_operation_is_reported_explicitly() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            assert!(!tokens.is_persistent(), "MCPFS_TOKEN_KEY is unset in this fixture");
            let (f, r) = token_set_registry(tokens).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(out["persistent"], false);
            let msg = out["message"].as_str().expect("a message explaining memory-only storage");
            assert!(
                msg.to_ascii_lowercase().contains("process")
                    || msg.to_ascii_lowercase().contains("restart"),
                "got {msg}"
            );
        });
    }

    #[test]
    fn e2e_new_191_an_rfc3339_expires_at_is_accepted_in_any_offset() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP,"expires_at":"2027-01-01T00:00:00Z"}),
            )
            .await
            .unwrap();
            let e1 = tokens.get_token(PERSON, "github.ibm.com").unwrap().expires_at.unwrap();

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({
                    "host":"github.ibm.com","token":GHP,"expires_at":"2027-01-01T01:00:00+01:00"
                }),
            )
            .await
            .unwrap();
            let e2 = tokens.get_token(PERSON, "github.ibm.com").unwrap().expires_at.unwrap();

            assert_eq!(e1, e2, "both offsets must store the same instant");
        });
    }

    #[test]
    fn e2e_new_192_a_non_rfc3339_expires_at_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            for bad in [json!(1_798_761_600i64), json!("tomorrow")] {
                let e = f
                    .call(
                        &r,
                        PERSON,
                        "git.token_set",
                        json!({"host":"github.ibm.com","token":GHP,"expires_at":bad}),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(e.code, code::INVALID_ARGUMENT, "{bad:?}");
                assert!(e.message.contains("expires_at"), "{}", e.message);
            }
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_193_a_past_expires_at_is_accepted_and_reports_expired() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let past = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP,"expires_at":past}),
            )
            .await
            .unwrap();

            assert!(
                !tokens.has_valid_token(PERSON, "github.ibm.com"),
                "an already-expired token reports invalid"
            );
            assert!(
                tokens.get_token(PERSON, "github.ibm.com").is_some(),
                "the session is still retrievable, only invalid"
            );
        });
    }

    #[test]
    fn e2e_new_221_a_failed_persistence_write_leaves_no_in_memory_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            db.arm();
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap_err();

            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
            assert!(!tokens.has_valid_token(PERSON, "github.ibm.com"));
        });
    }

    #[test]
    fn e2e_new_222_a_failed_write_restores_the_previous_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_OLD00000000000000000000000000"}),
            )
            .await
            .unwrap();

            db.arm();
            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_NEW00000000000000000000000000"}),
            )
            .await
            .unwrap_err();

            let s = tokens.get_token(PERSON, "github.ibm.com").unwrap();
            assert_eq!(s.access_token, "ghp_OLD00000000000000000000000000", "prior token intact");
        });
    }

    /// E2E-NEW-223: the device-flow poller shares the exact same rollback
    /// path as `git.token_set` (both call the one `store_token`), now keyed
    /// by the resolved host (the canonical default `github.com` here) rather
    /// than the bare provider name.
    #[test]
    fn e2e_new_223_the_device_flow_poller_rolls_back_identically() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            db.arm();
            let f = Fixture::with_config(|c| c.git.enabled = true).await;
            let flow = FakeFlow::new(vec![TokenPoll::granted(
                "gho_stored",
                vec!["repo".into()],
                future(),
            )]);
            let mut r = ToolRegistry::new();
            register_with(&mut r, Some(tokens.clone()), Some(flow.clone()));

            f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();

            assert!(eventually(|| flow.polls() >= 1).await, "the poller must run at least once");
            // Give the poller time to attempt (and fail) the armed persistence write.
            tokio::time::sleep(Duration::from_millis(150)).await;

            assert!(
                tokens.get_token(PERSON, "github.com").is_none(),
                "no in-memory token must remain after a failed persistence write"
            );
        });
    }

    // ── US-023: scope reporting on git.auth_status (FR-NEW-335) ─────────────

    /// Seed `(PERSON, host)` directly in the store with a known scope set.
    async fn seed(tokens: &OAuthTokenStore, host: &str, provider: &str, scopes: &[&str]) {
        tokens
            .store_token(
                PERSON,
                host,
                provider,
                &format!("tok_for_{host}"),
                scopes.iter().map(|s| s.to_string()).collect(),
                Some(future()),
                None,
            )
            .await
            .unwrap();
    }

    /// E2E-NEW-775: the GitLab flow's granted scopes land on the session and
    /// are reported verbatim, in order, as a valid entry.
    #[test]
    fn e2e_new_775_gitlab_granted_scopes_are_stored_and_reported() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::granted(
                "glpat_FAKE_999",
                vec!["api".into(), "read_repository".into(), "write_repository".into()],
                future(),
            )])
            .await;
            f.call(&r, PERSON, "git.auth", json!({"provider":"gitlab","host":"gitlab.com"}))
                .await
                .unwrap();
            assert!(eventually(|| tokens.has_valid_token(PERSON, "gitlab.com")).await);

            let out =
                f.call(&r, PERSON, "git.auth_status", json!({"host":"gitlab.com"})).await.unwrap();
            let e = &out["statuses"][0];
            assert_eq!(e["scopes"], json!(["api", "read_repository", "write_repository"]));
            assert_eq!(e["validity"], "valid");
            assert_eq!(e["pr_capable"], json!(true));
            assert_eq!(
                tokens.get_token(PERSON, "gitlab.com").unwrap().scopes,
                vec!["api", "read_repository", "write_repository"]
            );
        });
    }

    /// E2E-NEW-783: every entry keeps its five existing keys and gains the
    /// pull request capability report, per host.
    #[test]
    fn e2e_new_783_auth_status_reports_pr_capability_per_host() {
        with_git_hosts_lock(async {
            declare_hosts(&[
                ("github.com", "github"),
                ("gitlab.com", "gitlab"),
                ("gitlab.example.test", "gitlab"),
            ]);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            seed(&tokens, "github.com", "github", &["repo"]).await;
            seed(&tokens, "gitlab.com", "gitlab", &["read_repository", "write_repository"]).await;
            seed(&tokens, "gitlab.example.test", "gitlab", &["api"]).await;

            let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
            let st = out["statuses"].as_array().unwrap();
            assert_eq!(st.len(), 3);
            let hosts: Vec<&str> = st.iter().map(|e| e["host"].as_str().unwrap()).collect();
            assert_eq!(hosts, vec!["github.com", "gitlab.com", "gitlab.example.test"]);
            for e in st {
                for k in ["host", "provider", "validity", "expires_at", "scopes"] {
                    assert!(e.get(k).is_some(), "{k} must still be reported");
                }
            }
            assert_eq!((&st[0]["pr_read"], &st[0]["pr_write"]), (&json!(true), &json!(true)));
            assert_eq!((&st[1]["pr_read"], &st[1]["pr_write"]), (&json!(false), &json!(false)));
            assert_eq!((&st[2]["pr_read"], &st[2]["pr_write"]), (&json!(true), &json!(true)));
        });
    }

    /// E2E-NEW-784: `api` alone is reported as covering both halves of the
    /// surface; nothing requires `read_repository` alongside it.
    #[test]
    fn e2e_new_784_api_alone_is_reported_as_pr_capable() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            seed(&tokens, "gitlab.com", "gitlab", &["api"]).await;
            let out =
                f.call(&r, PERSON, "git.auth_status", json!({"host":"gitlab.com"})).await.unwrap();
            let e = &out["statuses"][0];
            assert_eq!(e["pr_read"], json!(true));
            assert_eq!(e["pr_write"], json!(true));
            assert_eq!(e["missing_scopes"], json!([]));
            // and the gate the pr tools will run agrees
            for access in [PrAccess::Read, PrAccess::Write] {
                tokens.require_pr_credential(PERSON, "gitlab.com", access).unwrap();
            }
        });
    }

    /// E2E-NEW-816: the scopes the flow granted are persisted verbatim and the
    /// resulting token passes the pull request gate with no pre-check
    /// rejection.
    #[test]
    fn e2e_new_816_granted_scopes_are_persisted_verbatim_and_pass_the_gate() {
        with_git_hosts_lock(async {
            declare_hosts(&[("gitlab.example.test", "gitlab")]);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::granted(
                "glpat_FAKE_816",
                vec!["api".into(), "read_repository".into(), "write_repository".into()],
                future(),
            )])
            .await;
            f.call(
                &r,
                PERSON,
                "git.auth",
                json!({"provider":"gitlab","instance_url":"https://gitlab.example.test"}),
            )
            .await
            .unwrap();
            assert!(eventually(|| tokens.has_valid_token(PERSON, "gitlab.example.test")).await);

            assert_eq!(
                tokens.get_token(PERSON, "gitlab.example.test").unwrap().scopes,
                vec!["api", "read_repository", "write_repository"]
            );
            let out = f
                .call(&r, PERSON, "git.auth_status", json!({"host":"gitlab.example.test"}))
                .await
                .unwrap();
            assert_eq!(
                out["statuses"][0]["scopes"],
                json!(["api", "read_repository", "write_repository"])
            );
            assert_eq!(out["statuses"][0]["pr_capable"], json!(true));
            assert_eq!(
                tokens
                    .require_pr_credential(PERSON, "gitlab.example.test", PrAccess::Read)
                    .unwrap(),
                "glpat_FAKE_816"
            );
        });
    }

    /// E2E-NEW-930 / E2E-NEW-781: an empty scope set is reported as unknown,
    /// distinguishable from both true and false, and is not refused by the
    /// gate; no token value appears anywhere in the response.
    #[test]
    fn e2e_new_930_an_empty_scope_set_is_reported_unknown_not_insufficient() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
            seed(&tokens, "github.com", "github", &["repo"]).await;
            seed(&tokens, "gitlab.com", "gitlab", &["read_repository", "write_repository"]).await;
            // seeded the way `git.token_set` does it: no scope information
            f.call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();

            let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
            let st = out["statuses"].as_array().unwrap();
            let by = |h: &str| st.iter().find(|e| e["host"] == h).unwrap().clone();

            let gh = by("github.com");
            assert_eq!(gh["scopes"], json!(["repo"]));
            assert_eq!(gh["pr_capable"], json!(true));

            let gl = by("gitlab.com");
            assert_eq!(gl["pr_capable"], json!(false));
            assert_eq!(gl["missing_scopes"], json!(["api"]));

            let ibm = by("github.ibm.com");
            assert_eq!(ibm["scopes"], json!([]));
            assert!(ibm["pr_capable"].is_null(), "unknown must not be reported as false");
            assert_eq!(ibm["missing_scopes"], json!([]));

            let body = serde_json::to_string(&out).unwrap();
            assert!(!body.contains("ghp_"), "no token value in the response");
            assert!(!body.contains("glpat"), "no token value in the response");
            assert!(!body.contains(GHP));

            // DEC-911: the unknown set is attempted, not pre-emptively refused.
            tokens.require_pr_credential(PERSON, "github.ibm.com", PrAccess::Write).unwrap();
        });
    }
}
