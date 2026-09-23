//! The seam every `git.pr_*` tool calls through (US-024).
//!
//! Three things live here and nowhere else:
//!
//! 1. [`resolve_target`], which turns a volume's remote URL into the provider,
//!    the REST API base URL and the `owner/repo` slug, using the already
//!    validated `git.hosts` map (FR-NEW-300/301/302). Going through that map is
//!    what makes GitHub Enterprise Server and self hosted GitLab work with no
//!    new configuration.
//! 2. [`ProviderClient`], a trait so a test substitutes a fake returning canned
//!    provider JSON (FR-NEW-315). Mirrors the device flow's
//!    [`DeviceFlowClient`](crate::git::oauth::device_flow::DeviceFlowClient)
//!    seam: without it the whole pull request surface is untestable offline.
//! 3. [`HttpProviderClient`], the only real transport, which owns the transport
//!    safety rules: no cross host redirect is ever followed (FR-NEW-316), a
//!    bounded response body, a configured timeout, and a credential that never
//!    reaches a log line, a span field or a returned body (FR-NEW-314).
//!
//! The credential itself is NOT resolved here: `OAuthTokenStore::require_pr_credential`
//! owns the token lookup plus the scope gate (US-023), and the tool layer calls
//! it, then hands the resulting token to [`Credential::new`].

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

pub mod model;

use crate::config::GitConfig;
use crate::errors::{Result, ToolError};
use crate::git::remote::{Provider, extract_host, resolve_host, validate_remote_url};

/// What the server sends as `User-Agent` on every provider REST call. The same
/// literal the device flow uses, so one deployment presents one identity.
const USER_AGENT: &str = concat!("mcp-fs/", env!("CARGO_PKG_VERSION"));

/// Hard ceiling on a provider JSON response. A refusal, not a truncation: a
/// half read JSON document cannot be parsed, so returning a prefix would only
/// turn a size problem into a parse error.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// How many same host redirects are followed before the call is refused. A
/// bound exists because a provider that redirects in a loop would otherwise
/// hold the request thread for the whole timeout.
const MAX_REDIRECTS: usize = 5;

/// The two providers that expose a pull request REST API this server speaks.
/// `generic` and `anonymous` have no API contract to normalize, so they are
/// rejected at resolution time (FR-NEW-301).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrProvider {
    Github,
    Gitlab,
}

impl PrProvider {
    /// The label a span field, an audit entry and an error message all use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
        }
    }
}

/// Everything a `git.pr_*` call needs before it may touch the network: which
/// provider, which API base, and which project the remote points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTarget {
    /// The remote's bare hostname, as declared in `git.hosts`.
    pub host: String,
    pub provider: PrProvider,
    /// REST API base, no trailing slash. `https://api.github.com`,
    /// `https://github.ibm.com/api/v3`, `https://gitlab.example.test/api/v4`.
    pub base_url: String,
    pub owner: String,
    pub repo: String,
}

impl ProviderTarget {
    /// `owner/repo`, the form both providers' path segments are built from.
    pub fn project_path(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The host of [`Self::base_url`], which is what a redirect is compared
    /// against. Falls back to [`Self::host`] only if the base could not be
    /// parsed, which cannot happen for a base this module built.
    pub fn api_host(&self) -> String {
        url::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| self.host.clone())
    }
}

/// Resolve the provider, the API base and the project slug from a volume's
/// remote URL (FR-NEW-300). Every failure here happens before any network call
/// (FR-NEW-302) and before any token is read (FR-NEW-301 ordering).
///
/// `instance_url` is the value stored alongside the person's token for the
/// host, used for a self hosted GitLab whose API lives somewhere other than the
/// git remote's own origin. It is ignored for GitHub, whose Enterprise API is
/// always `/api/v3` on the same host.
pub fn resolve_target(remote_url: &str, instance_url: Option<&str>) -> Result<ProviderTarget> {
    // `validate_remote_url` is the single URL gate in the tree: it names the scp
    // shorthand, a non https scheme and an embedded credential with the exact
    // messages the remote tools already produce.
    let parsed = validate_remote_url(remote_url)?;
    let host = extract_host(remote_url);
    let provider = match resolve_host(&host)? {
        Provider::Github => PrProvider::Github,
        Provider::Gitlab => PrProvider::Gitlab,
        Provider::Generic | Provider::Anonymous => {
            return Err(ToolError::not_supported(format!(
                "host '{host}' resolves to a provider with no pull request API; pull request \
                 operations require a 'github' or 'gitlab' provider in git.hosts"
            )));
        }
    };
    let (owner, repo) = project_slug(remote_url, &parsed)?;
    let base_url = base_url_for(provider, &host, instance_url);
    Ok(ProviderTarget { host, provider, base_url, owner, repo })
}

/// `owner/repo` from the URL path, with the conventional `.git` suffix removed.
/// A path with fewer than two non empty segments names no repository, so it is
/// rejected naming the URL and the missing part (E2E-NEW-924).
fn project_slug(raw: &str, parsed: &url::Url) -> Result<(String, String)> {
    let segments: Vec<&str> =
        parsed.path().split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>();
    if segments.len() < 2 {
        return Err(ToolError::invalid_argument(format!(
            "remote url '{raw}' names no repository: expected a path of the form \
             '/owner/repository'"
        )));
    }
    let owner = segments[..segments.len() - 1].join("/");
    let repo = segments[segments.len() - 1].trim_end_matches(".git").to_string();
    if repo.is_empty() {
        return Err(ToolError::invalid_argument(format!(
            "remote url '{raw}' names no repository: the last path segment is empty"
        )));
    }
    Ok((owner, repo))
}

/// github.com answers on `api.github.com`; every other GitHub host is an
/// Enterprise Server, whose API is `/api/v3` on the host itself. GitLab is
/// always `/api/v4`, on the stored `instance_url` when one exists (a self
/// hosted deployment whose API base differs from the git origin) and on the
/// remote's own host otherwise.
fn base_url_for(provider: PrProvider, host: &str, instance_url: Option<&str>) -> String {
    match provider {
        PrProvider::Github if host.eq_ignore_ascii_case("github.com") => {
            "https://api.github.com".to_string()
        }
        PrProvider::Github => format!("https://{host}/api/v3"),
        PrProvider::Gitlab => {
            let base = instance_url
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| format!("https://{host}"));
            format!("{base}/api/v4")
        }
    }
}

/// One person's credential for one host, already scope checked by
/// `OAuthTokenStore::require_pr_credential`.
///
/// The token is private and only ever leaves through [`Self::header_value`],
/// which the transport hands straight to the `Authorization` header, and
/// [`Self::redact`], which scrubs it out of anything that would be returned.
#[derive(Clone)]
pub struct Credential {
    provider: PrProvider,
    token: String,
}

impl Credential {
    pub fn new(provider: PrProvider, token: impl Into<String>) -> Self {
        Self { provider, token: token.into() }
    }

    /// GitHub's REST API takes `token <t>`, GitLab's takes `Bearer <t>`.
    pub fn header_value(&self) -> String {
        match self.provider {
            PrProvider::Github => format!("token {}", self.token),
            PrProvider::Gitlab => format!("Bearer {}", self.token),
        }
    }

    /// Replace the token wherever it appears. Applied to every error message
    /// and every response body the transport returns, because a provider that
    /// echoes the credential back in a 401 body would otherwise leak it
    /// straight into a tool response (FR-NEW-314).
    pub fn redact(&self, text: &str) -> String {
        if self.token.is_empty() || !text.contains(&self.token) {
            return text.to_string();
        }
        text.replace(&self.token, "<redacted>")
    }

    fn redact_bytes(&self, body: Vec<u8>) -> Vec<u8> {
        if self.token.is_empty() {
            return body;
        }
        let needle = self.token.as_bytes();
        if body.windows(needle.len()).any(|w| w == needle) {
            return String::from_utf8_lossy(&body).replace(&self.token, "<redacted>").into_bytes();
        }
        body
    }
}

/// Never prints the token: a credential must be safe to put in any diagnostic.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("provider", &self.provider.label())
            .field("token", &"<redacted>")
            .finish()
    }
}

/// The HTTP verbs the pull request surface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// One provider REST call. Carries no credential: the transport takes that
/// separately, so a request is safe to log and a fake client can record it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequest {
    pub method: Method,
    /// API base, no trailing slash. [`ProviderTarget::base_url`].
    pub base_url: String,
    /// Absolute path on the base, leading slash included.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
    /// The remote's host, used only to name the offender in the oversize
    /// refusal, which is about the deployment the caller configured rather
    /// than about the API endpoint that answered.
    pub origin_host: String,
    /// `Accept` override, for the endpoints that negotiate a diff or patch
    /// media type instead of JSON.
    pub accept: Option<String>,
    /// Stop reading the body at this many bytes and report the answer as
    /// truncated instead of refusing it. Set only by `git.pr_diff`, the one
    /// endpoint whose answer is a text stream whose prefix is still useful; a
    /// JSON endpoint leaves it `None`, because half a JSON document cannot be
    /// parsed (FR-NEW-309).
    pub truncate_at: Option<usize>,
}

impl ProviderRequest {
    pub fn new(target: &ProviderTarget, method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            base_url: target.base_url.clone(),
            path: path.into(),
            query: Vec::new(),
            body: None,
            origin_host: target.host.clone(),
            accept: None,
            truncate_at: None,
        }
    }

    #[must_use]
    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }

    #[must_use]
    pub fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    #[must_use]
    pub fn accept(mut self, accept: impl Into<String>) -> Self {
        self.accept = Some(accept.into());
        self
    }

    #[must_use]
    pub fn truncate_at(mut self, bytes: usize) -> Self {
        self.truncate_at = Some(bytes);
        self
    }

    fn url(&self) -> Result<url::Url> {
        let joined = format!("{}{}", self.base_url.trim_end_matches('/'), self.path);
        let mut parsed = url::Url::parse(&joined).map_err(|e| {
            ToolError::internal(format!("provider api url '{joined}' is not valid: {e}"))
        })?;
        if !self.query.is_empty() {
            let mut pairs = parsed.query_pairs_mut();
            for (k, v) in &self.query {
                pairs.append_pair(k, v);
            }
        }
        Ok(parsed)
    }
}

/// What a provider answered. The body is raw bytes, already scrubbed of the
/// credential, so a caller may parse it as JSON or hand it on as a diff.
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub status: u16,
    pub body: Vec<u8>,
    /// The answer's `Content-Type`, lowercased and without its parameters.
    /// Needed because `git.pr_diff` must tell a unified diff from the HTML
    /// error page a proxy may answer with under a 200 (FR-NEW-309).
    pub content_type: Option<String>,
    /// The body stopped at [`ProviderRequest::truncate_at`] with more bytes
    /// still available.
    pub truncated: bool,
}

impl ProviderResponse {
    /// A complete answer whose content type was not recorded: what every JSON
    /// endpoint produces, and what a fake transport builds.
    pub fn complete(status: u16, body: Vec<u8>) -> Self {
        Self { status, body, content_type: None, truncated: false }
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.body).map_err(|e| {
            ToolError::internal(format!("provider returned an unreadable JSON body: {e}"))
        })
    }
}

/// Provider REST transport. A trait so a test injects a fake returning canned
/// provider JSON and never reaches the network (FR-NEW-315).
#[async_trait]
pub trait ProviderClient: Send + Sync {
    async fn send(&self, req: &ProviderRequest, cred: &Credential) -> Result<ProviderResponse>;
}

/// The real transport, and the only place the `Authorization` header is set.
///
/// One `reqwest::Client` per process: it owns the connection pool, so building
/// one per request would discard every kept alive TLS connection.
pub struct HttpProviderClient {
    http: reqwest::Client,
}

impl HttpProviderClient {
    pub fn new(timeout_secs: u64) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            // Redirects are handled by hand below rather than by reqwest:
            // reqwest's own policy would decide before this code can compare
            // hosts, and letting it follow would replay the Authorization
            // header to whatever host the response named (FR-NEW-316).
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| ToolError::internal(format!("provider http client build failed: {e}")))?;
        Ok(Self { http })
    }

    /// Read at most [`MAX_RESPONSE_BYTES`], refusing as soon as the limit is
    /// passed instead of buffering the whole body first. That is what keeps a
    /// hostile 64 MiB answer from being fully downloaded and parsed.
    async fn read_capped(
        mut resp: reqwest::Response,
        origin_host: &str,
        truncate_at: Option<usize>,
    ) -> Result<(u16, Vec<u8>, bool)> {
        let status = resp.status().as_u16();
        let mut body = Vec::new();
        let mut truncated = false;
        loop {
            // Tested before the next chunk is asked for, so a declared cap
            // really stops the download instead of trimming a body that was
            // already pulled in whole.
            if truncate_at.is_some_and(|cap| body.len() >= cap) {
                truncated = true;
                break;
            }
            let chunk = resp.chunk().await.map_err(|e| {
                ToolError::internal(format!(
                    "provider response from host '{origin_host}' \
                     could not be read: {e}"
                ))
            })?;
            match chunk {
                Some(c) => {
                    if let Some(cap) = truncate_at {
                        // A capped read never refuses: it keeps the prefix and
                        // says so, which is what makes a huge diff usable.
                        let room = cap.saturating_sub(body.len());
                        if c.len() > room {
                            body.extend_from_slice(&c[..room]);
                            truncated = true;
                            break;
                        }
                        body.extend_from_slice(&c);
                        continue;
                    }
                    if body.len() + c.len() > MAX_RESPONSE_BYTES {
                        return Err(ToolError::invalid_argument(format!(
                            "response from host '{origin_host}' exceeds the 16 MiB limit"
                        )));
                    }
                    body.extend_from_slice(&c);
                }
                None => break,
            }
        }
        Ok((status, body, truncated))
    }
}

#[async_trait]
impl ProviderClient for HttpProviderClient {
    async fn send(&self, req: &ProviderRequest, cred: &Credential) -> Result<ProviderResponse> {
        let mut url = req.url()?;
        // The host every redirect is compared against: the host of the API base
        // the caller resolved, never the host of the previous hop, so a chain of
        // same host redirects cannot walk away one hop at a time.
        let api_host = url
            .host_str()
            .ok_or_else(|| ToolError::internal("provider api url has no host".to_string()))?
            .to_string();

        for _ in 0..=MAX_REDIRECTS {
            // The span carries the routing facts and nothing else: no header,
            // no credential, no body (FR-NEW-314).
            let span = tracing::info_span!(
                "git.provider_request",
                method = req.method.as_str(),
                host = %api_host,
                path = %url.path(),
            );
            let _enter = span.enter();

            let mut builder = match req.method {
                Method::Get => self.http.get(url.clone()),
                Method::Post => self.http.post(url.clone()),
                Method::Put => self.http.put(url.clone()),
                Method::Patch => self.http.patch(url.clone()),
                Method::Delete => self.http.delete(url.clone()),
            }
            .header(reqwest::header::AUTHORIZATION, cred.header_value())
            .header(
                reqwest::header::ACCEPT,
                req.accept.clone().unwrap_or_else(|| "application/json".to_string()),
            );
            if let Some(body) = &req.body {
                builder = builder.json(body);
            }

            let resp = builder.send().await.map_err(|e| {
                let what = if e.is_timeout() { "timed out" } else { "failed" };
                ToolError::internal(
                    cred.redact(&format!("provider request to host '{api_host}' {what}: {e}")),
                )
            })?;

            let status = resp.status();
            if !status.is_redirection() {
                drop(_enter);
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.split(';').next().unwrap_or_default().trim().to_ascii_lowercase());
                let (status, body, truncated) =
                    HttpProviderClient::read_capped(resp, &req.origin_host, req.truncate_at)
                        .await
                        .map_err(|e| ToolError::new(e.code, cred.redact(&e.message)))?;
                return Ok(ProviderResponse {
                    status,
                    body: cred.redact_bytes(body),
                    content_type,
                    truncated,
                });
            }

            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    ToolError::internal(format!(
                        "provider at host '{api_host}' answered HTTP {} with no usable Location \
                         header",
                        status.as_u16()
                    ))
                })?
                .to_string();
            let next = url.join(&location).map_err(|e| {
                ToolError::internal(format!(
                    "provider at host '{api_host}' answered a redirect whose Location is not a \
                     valid URL: {e}"
                ))
            })?;
            let next_host = next.host_str().unwrap_or_default().to_string();
            if !next_host.eq_ignore_ascii_case(&api_host) {
                // Never re-issue: following this would put the Authorization
                // header on a host the response chose (FR-NEW-316).
                return Err(ToolError::internal(format!(
                    "refused redirect from host '{api_host}' to '{next_host}'"
                )));
            }
            url = next;
        }

        Err(ToolError::internal(format!(
            "provider at host '{api_host}' redirected more than {MAX_REDIRECTS} times"
        )))
    }
}

/// Process wide provider client, built once so the connection pool is reused
/// across every `git.pr_*` call. Mirrors `tools::git_auth`'s device flow
/// singleton, including caching the build failure: a client that cannot be
/// built must fail loudly every time rather than be rebuilt per request.
static CLIENT: OnceLock<Result<Arc<dyn ProviderClient>>> = OnceLock::new();

pub fn shared_client(config: &GitConfig) -> Result<Arc<dyn ProviderClient>> {
    CLIENT
        .get_or_init(|| {
            HttpProviderClient::new(config.provider_api_timeout_secs)
                .map(|c| Arc::new(c) as Arc<dyn ProviderClient>)
        })
        .clone()
}

#[cfg(test)]
mod tests;
