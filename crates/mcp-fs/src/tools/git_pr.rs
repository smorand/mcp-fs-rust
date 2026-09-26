//! `git.pr_*` tools: the pull request surface, over the provider REST API.
//!
//! One shape in, one shape out. Every tool here:
//!
//! 1. runs `state.authorize(mount_id, person)` first, membership only, no
//!    platform admin bypass,
//! 2. resolves the declared remote through `git::remote::require_remote`, then
//!    the provider through [`resolve_target`], both offline,
//! 3. passes the scope gate `OAuthTokenStore::require_pr_credential`, the single
//!    call that does the token lookup AND the scope validation (US-023), before
//!    ANY network call, pre-flights included,
//! 4. calls the provider through the injected [`ProviderClient`], and
//! 5. returns [`PullRequest`], the one normalized model (FR-NEW-303/317).
//!
//! A tool never hand builds the response keys: that is exactly the drift this
//! specification kept re-discovering, so the key set lives in one struct.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::errors::{Result, ToolError};
use crate::git::GitRepoStore;
use crate::git::oauth::scopes::PrAccess;
use crate::git::oauth::store::OAuthTokenStore;
use crate::git::provider::model::{PrSignals, PullRequest};
use crate::git::provider::{
    Credential, Method, PrProvider, ProviderClient, ProviderRequest, ProviderResponse,
    ProviderTarget, resolve_target, shared_client,
};
use crate::mcp::registry::{ToolCtx, handler};
use crate::mcp::{ToolRegistry, ToolSchema};

/// Register the `git.pr_*` tools.
pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, None, None, None);
}

/// Registration with injected dependencies, for tests: a git store that is not
/// the process singleton, a token store holding seeded credentials, and a fake
/// provider API that returns canned JSON and never reaches the network.
pub fn register_with(
    reg: &mut ToolRegistry,
    git: Option<Arc<GitRepoStore>>,
    tokens: Option<Arc<OAuthTokenStore>>,
    api: Option<Arc<dyn ProviderClient>>,
) {
    let (git2, tokens2, api2) = (git.clone(), tokens.clone(), api.clone());
    let (git3, tokens3, api3) = (git.clone(), tokens.clone(), api.clone());
    let (git4, tokens4, api4) = (git.clone(), tokens.clone(), api.clone());
    let (git5, tokens5, api5) = (git.clone(), tokens.clone(), api.clone());
    let (git6, tokens6, api6) = (git.clone(), tokens.clone(), api.clone());
    let (g, t, c) = (git, tokens, api);
    reg.add(
        ToolSchema::new(
            "git.pr_create",
            "Open a pull request on GitHub, or a merge request on GitLab, for a declared remote. \
             Fails before contacting the provider when the head branch is not on the remote yet.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("base", "Branch the pull request merges into, on the remote.")
        .req_str("head", "Branch holding the changes; it must already exist on the remote.")
        .req_str("title", "Title of the pull request.")
        .opt_str_null("body", "Description of the pull request.")
        .opt_bool(
            "draft",
            false,
            "Open the pull request as a draft; on GitLab, which has no draft flag, the title is \
             prefixed with 'Draft: '.",
        )
        .opt_str(
            "remote",
            "origin",
            "Name of the declared remote to open it on; defaults to origin.",
        )
        .destructive(false)
        .read_only(false)
        .idempotent(false)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (g, t, c) = (g.clone(), t.clone(), c.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let base = a.str("base")?;
                let head = a.str("head")?;
                let title = a.str("title")?;
                let body = a.opt_str("body").unwrap_or_default();
                let draft = a.bool_or("draft", false);
                let remote = a
                    .opt_str("remote")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "origin".to_string());
                // Both refusals are pure argument checks, so they run before
                // anything is resolved and cost no call at all.
                if title.trim().is_empty() {
                    return Err(ToolError::invalid_argument(
                        "title must not be empty or whitespace-only",
                    ));
                }
                if base == head {
                    return Err(ToolError::invalid_argument(format!(
                        "base and head must differ, both are '{base}'"
                    )));
                }
                let call = PrCall::open(
                    &ctx,
                    &mount_id,
                    &remote,
                    PrAccess::Write,
                    "git.pr_create",
                    g,
                    t,
                    c,
                )
                .await?;
                call.create(&base, &head, &title, &body, draft).await
            }
        }),
    );

    let (g, t, c) = (git2, tokens2, api2);
    reg.add(
        ToolSchema::new(
            "git.pr_list",
            "List the pull requests of a declared remote on GitHub, or its merge requests on \
             GitLab, filtered by state and normalized to one shape.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .opt_str(
            "state",
            "open",
            "Which pull requests to list: open, closed, merged or all; defaults to open.",
        )
        .opt_str(
            "remote",
            "origin",
            "Name of the declared remote to list from; defaults to origin.",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (g, t, c) = (g.clone(), t.clone(), c.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let remote = a
                    .opt_str("remote")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "origin".to_string());
                // A pure argument check, so an unsupported state costs no
                // lookup and no call at all (FR-NEW-306).
                let state = PrState::parse(a.opt_str("state").as_deref().unwrap_or("open"))?;
                let call =
                    PrCall::open(&ctx, &mount_id, &remote, PrAccess::Read, "git.pr_list", g, t, c)
                        .await?;
                call.list(state).await
            }
        }),
    );

    let (g, t, c) = (git3, tokens3, api3);
    reg.add(
        ToolSchema::new(
            "git.pr_get",
            "Read one pull request on GitHub, or one merge request on GitLab, normalized to one \
             shape and enriched with its review state and its check state. Fails rather than \
             reporting an unknown review or check state as none.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_int("pr_number", "Number of the pull request, as the provider numbers it.")
        .opt_str(
            "remote",
            "origin",
            "Name of the declared remote to read it from; defaults to origin.",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (g, t, c) = (g.clone(), t.clone(), c.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let number = pr_number(&a)?;
                let remote = a
                    .opt_str("remote")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "origin".to_string());
                let call =
                    PrCall::open(&ctx, &mount_id, &remote, PrAccess::Read, "git.pr_get", g, t, c)
                        .await?;
                call.get(number).await
            }
        }),
    );

    let (g, t, c) = (git4, tokens4, api4);
    reg.add(
        ToolSchema::new(
            "git.pr_diff",
            "Read the unified diff of one pull request on GitHub, or one merge request on GitLab. \
             The answer is bounded by git.max_pr_diff_mb; a larger diff is returned truncated, \
             with truncated set to true.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_int("pr_number", "Number of the pull request, as the provider numbers it.")
        .opt_str(
            "remote",
            "origin",
            "Name of the declared remote to read it from; defaults to origin.",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (g, t, c) = (g.clone(), t.clone(), c.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let number = pr_number(&a)?;
                let remote = a
                    .opt_str("remote")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "origin".to_string());
                // Read before the gate order runs, so the cap is the deployment's
                // and never a caller supplied one.
                let cap = ctx.state.config.git.max_pr_diff_mb;
                let call =
                    PrCall::open(&ctx, &mount_id, &remote, PrAccess::Read, "git.pr_diff", g, t, c)
                        .await?;
                call.diff(number, cap).await
            }
        }),
    );

    let (g, t, c) = (git5, tokens5, api5);
    reg.add(
        ToolSchema::new(
            "git.pr_merge",
            "Merge a pull request on GitHub, or a merge request on GitLab, with the chosen \
             strategy. The merge happens on the provider: no local ref and no remote-tracking ref \
             is updated, so git.remote_fetch is required to observe it locally. A refusal by the \
             provider (failing checks, missing reviews, a protected branch, a strategy disabled \
             for that repository, an already merged or closed pull request) is reported as the \
             provider's own status and message, never as a success.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_int("pr_number", "Number of the pull request, as the provider numbers it.")
        .req_str(
            "strategy",
            "How the provider must merge it: merge, squash or rebase; the word is mapped to each \
             provider's own spelling.",
        )
        .opt_str_null(
            "commit_title",
            "Title of the resulting commit; the provider's own default is used when absent.",
        )
        .opt_str_null(
            "commit_message",
            "Body of the resulting commit; the provider's own default is used when absent.",
        )
        .opt_str("remote", "origin", "Name of the declared remote to merge on; defaults to origin.")
        .destructive(true)
        .read_only(false)
        .idempotent(false)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (g, t, c) = (g.clone(), t.clone(), c.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let number = pr_number(&a)?;
                // A pure argument check, exactly like the `state` filter of
                // git.pr_list: an unsupported strategy costs no lookup and no
                // call at all (FR-NEW-310).
                let strategy = MergeStrategy::parse(&a.str("strategy")?)?;
                let title = a.opt_str("commit_title").filter(|s| !s.is_empty());
                let message = a.opt_str("commit_message").filter(|s| !s.is_empty());
                let remote = a
                    .opt_str("remote")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "origin".to_string());
                let call = PrCall::open(
                    &ctx,
                    &mount_id,
                    &remote,
                    PrAccess::Write,
                    "git.pr_merge",
                    g,
                    t,
                    c,
                )
                .await?;
                call.merge(number, strategy, title.as_deref(), message.as_deref()).await
            }
        }),
    );

    let (g, t, c) = (git6, tokens6, api6);
    reg.add(
        ToolSchema::new(
            "git.pr_review",
            "Submit a review verdict on a pull request on GitHub, or on a merge request on \
             GitLab: approve it, request changes, or comment. On GitLab, which has no review \
             verdict, approve uses the approve endpoint while the other two leave a note. A \
             refusal by the provider (reviewing one's own pull request, a token the provider no \
             longer accepts) is reported as the provider's own status and message, never as a \
             success.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_int("pr_number", "Number of the pull request, as the provider numbers it.")
        .req_str(
            "verdict",
            "The verdict to submit: approve, request_changes or comment; the word is mapped to \
             each provider's own spelling.",
        )
        .opt_str_null(
            "body",
            "Text of the review; required and non-empty for request_changes and comment, \
             optional for approve.",
        )
        .opt_str(
            "remote",
            "origin",
            "Name of the declared remote to review on; defaults to origin.",
        )
        .destructive(false)
        .read_only(false)
        .idempotent(false)
        .open_world(true),
        handler(move |ctx: ToolCtx, a| {
            let (g, t, c) = (g.clone(), t.clone(), c.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let number = pr_number(&a)?;
                // Both checks are pure argument checks, exactly like the
                // `strategy` of git.pr_merge: an unsupported verdict and a
                // verdict with no reasoning cost no lookup and no call at all.
                let verdict = Verdict::parse(&a.str("verdict")?)?;
                let body = a.opt_str("body").unwrap_or_default();
                if verdict.requires_body() && body.trim().is_empty() {
                    return Err(ToolError::invalid_argument(format!(
                        "verdict '{}' requires a non-empty body",
                        verdict.label()
                    )));
                }
                let remote = a
                    .opt_str("remote")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "origin".to_string());
                let call = PrCall::open(
                    &ctx,
                    &mount_id,
                    &remote,
                    PrAccess::Write,
                    "git.pr_review",
                    g,
                    t,
                    c,
                )
                .await?;
                call.review(number, verdict, &body).await
            }
        }),
    );
}

/// A pull request number is a positive integer: a zero or negative one names no
/// pull request on either provider, so it is refused before any lookup.
fn pr_number(a: &crate::mcp::args::Args) -> Result<u64> {
    let raw = a.int("pr_number")?;
    u64::try_from(raw).ok().filter(|n| *n > 0).ok_or_else(|| {
        ToolError::invalid_argument(format!("pr_number must be a positive integer, got {raw}"))
    })
}

/// The `state` filter, parsed once so no call site compares raw strings.
///
/// The two providers spell their filters differently, so the caller's word is
/// mapped rather than passed through: GitLab says `opened` where GitHub says
/// `open`, and GitHub has no merged filter at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrState {
    Open,
    Closed,
    Merged,
    All,
}

impl PrState {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "merged" => Ok(Self::Merged),
            "all" => Ok(Self::All),
            other => Err(ToolError::invalid_argument(format!(
                "state must be one of open, closed, merged, all, got '{other}'"
            ))),
        }
    }

    /// GitHub's `state` accepts `open`, `closed` and `all` only: a merged pull
    /// request is a closed one, and telling them apart is the client's job.
    fn github(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed | Self::Merged => "closed",
            Self::All => "all",
        }
    }

    fn gitlab(self) -> &'static str {
        match self {
            Self::Open => "opened",
            Self::Closed => "closed",
            Self::Merged => "merged",
            Self::All => "all",
        }
    }
}

/// The merge strategy, parsed once so no call site compares raw strings.
///
/// The caller's word is mapped, never passed through: GitHub names a
/// `merge_method`, GitLab has no such field at all and expresses the same three
/// intents with a `squash` flag plus a separate rebase endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

impl MergeStrategy {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "merge" => Ok(Self::Merge),
            "squash" => Ok(Self::Squash),
            "rebase" => Ok(Self::Rebase),
            other => Err(ToolError::invalid_argument(format!(
                "strategy must be one of merge, squash, rebase, got '{other}'"
            ))),
        }
    }

    /// The caller's word, for the error messages: a refusal must name the
    /// strategy that was refused, in the caller's own vocabulary.
    fn label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }

    /// GitHub's `merge_method`, which happens to spell the three the same way.
    fn github(self) -> &'static str {
        self.label()
    }
}

/// The review verdict, parsed once so no call site compares raw strings.
///
/// The caller's word is mapped, never passed through: GitHub names a review
/// `event`, and GitLab has no review verdict at all, expressing the same three
/// intents with an approve endpoint and a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Approve,
    RequestChanges,
    Comment,
}

impl Verdict {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "approve" => Ok(Self::Approve),
            "request_changes" => Ok(Self::RequestChanges),
            "comment" => Ok(Self::Comment),
            other => Err(ToolError::invalid_argument(format!(
                "verdict must be one of approve, request_changes, comment, got '{other}'"
            ))),
        }
    }

    /// The caller's own word, for the error messages and the response.
    fn label(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
            Self::Comment => "comment",
        }
    }

    /// An approval says everything by itself; the other two are a demand on the
    /// author, and one with no reasoning is useless to them.
    fn requires_body(self) -> bool {
        self != Self::Approve
    }

    /// GitHub's review `event`.
    fn github(self) -> &'static str {
        match self {
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
            Self::Comment => "COMMENT",
        }
    }

    /// The review state this verdict leaves behind, in the normalized
    /// vocabulary of FR-NEW-307. A comment is not a verdict: someone looked,
    /// nobody decided, so the pull request still needs a review.
    fn review_state(self) -> &'static str {
        match self {
            Self::Approve => "approved",
            Self::RequestChanges => "changes_requested",
            Self::Comment => "review_required",
        }
    }
}

/// Everything a `git.pr_*` tool holds once the offline gates have passed: which
/// project it talks about, the resolved target, the scope checked credential and
/// the transport. Built by [`PrCall::open`], which IS the gate order.
struct PrCall {
    target: ProviderTarget,
    cred: Credential,
    client: Arc<dyn ProviderClient>,
    tool: &'static str,
}

impl PrCall {
    #[allow(clippy::too_many_arguments)]
    async fn open(
        ctx: &ToolCtx,
        mount_id: &str,
        remote: &str,
        access: PrAccess,
        tool: &'static str,
        git: Option<Arc<GitRepoStore>>,
        tokens: Option<Arc<OAuthTokenStore>>,
        api: Option<Arc<dyn ProviderClient>>,
    ) -> Result<Self> {
        // Membership first, before anything reads project state.
        ctx.state.authorize(mount_id, &ctx.person).await?;
        let store = git.unwrap_or_else(|| {
            GitRepoStore::shared(ctx.state.config.clone(), ctx.state.stores.relational().clone())
        });
        let url = crate::git::remote::require_remote(&store, mount_id, remote).await?;
        // Offline: provider, API base and owner/repo, all from the declared URL.
        let mut target = resolve_target(&url, None)?;

        let tokens = match tokens {
            Some(t) => t,
            None => {
                super::git_auth::token_store(&ctx.state.config, ctx.state.stores.relational())
                    .await?
            }
        };
        // The scope gate, before every network call including the pre-flights.
        // The tool name is added here because the store's message is shared by
        // the whole family and a caller must see which tool was refused.
        let token = tokens
            .require_pr_credential(&ctx.person, &target.host, access)
            .map_err(|e| ToolError::new(e.code, format!("{tool}: {}", e.message)))?;

        // A self hosted GitLab may serve its API somewhere other than the git
        // origin, and the base recorded with the person's token is the only
        // place that is known. Read after the gate, so a caller with no right
        // to the host learns nothing about how it is deployed.
        if target.provider == PrProvider::Gitlab
            && let Some(instance) =
                tokens.get_token(&ctx.person, &target.host).and_then(|s| s.instance_url)
        {
            target = resolve_target(&url, Some(&instance))?;
        }

        let client = match api {
            Some(c) => c,
            None => shared_client(&ctx.state.config.git)?,
        };
        let cred = Credential::new(target.provider, token);
        Ok(Self { target, cred, client, tool })
    }

    /// FR-NEW-304/305: reject an absent head branch before the create request,
    /// then create and normalize.
    async fn create(
        &self,
        base: &str,
        head: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<Value> {
        self.require_head_on_remote(head).await?;
        let (path, payload) = match self.target.provider {
            PrProvider::Github => (
                format!("/repos/{}/pulls", self.target.project_path()),
                // Key order is the provider's own documented order, and the
                // `draft` flag is always explicit: GitHub defaults it to false
                // anyway, and stating it keeps the request self describing.
                json!({"base": base, "head": head, "title": title, "body": body,
                       "draft": draft}),
            ),
            PrProvider::Gitlab => {
                // GitLab has no draft flag: the marker is a title prefix, which
                // the normalizer strips back off so both providers agree on
                // `title` (E2E-NEW-721).
                let title = if draft { format!("Draft: {title}") } else { title.to_string() };
                (
                    format!("/projects/{}/merge_requests", self.encoded_project()),
                    json!({"source_branch": head, "target_branch": base, "title": title,
                           "description": body}),
                )
            }
        };
        let resp =
            self.send(ProviderRequest::new(&self.target, Method::Post, path).body(payload)).await?;
        if !resp.is_success() {
            // A duplicate pull request, a protected branch, a permission the
            // token does not carry: all of them are the provider's answer and
            // are reported as such, never swallowed into a success.
            return Err(self.provider_error("create the pull request", &resp));
        }
        Ok(self.normalize(resp.json()?, &PrSignals::unknown()))
    }

    /// FR-NEW-305: the head branch must already be on the remote, and it is
    /// checked with a read before the create is issued, so a caller who forgot
    /// to push gets the remedy instead of a provider side error.
    async fn require_head_on_remote(&self, head: &str) -> Result<()> {
        let path = match self.target.provider {
            PrProvider::Github => {
                format!("/repos/{}/branches/{head}", self.target.project_path())
            }
            PrProvider::Gitlab => {
                format!("/projects/{}/repository/branches/{head}", self.encoded_project())
            }
        };
        let resp = self.send(ProviderRequest::new(&self.target, Method::Get, path)).await?;
        if resp.is_success() {
            return Ok(());
        }
        if resp.status == 404 {
            return Err(ToolError::not_found(format!(
                "head branch '{head}' does not exist on host '{}' for '{}'; push it first with \
                 git.remote_push",
                self.target.host,
                self.target.project_path()
            )));
        }
        Err(self.provider_error("check the head branch", &resp))
    }

    /// FR-NEW-306: every pull request matching `state`, as list items.
    ///
    /// Pages are walked until one comes back short of [`PAGE_SIZE`], which is
    /// how both providers signal the end. The `Link` header would say the same
    /// thing on GitHub only, so the page size rule is used for both rather than
    /// carrying a provider specific header through the transport.
    async fn list(&self, state: PrState) -> Result<Value> {
        let (path, filter) = match self.target.provider {
            PrProvider::Github => {
                (format!("/repos/{}/pulls", self.target.project_path()), state.github())
            }
            PrProvider::Gitlab => {
                (format!("/projects/{}/merge_requests", self.encoded_project()), state.gitlab())
            }
        };
        // GitHub answers `merged` with every closed pull request, so the ones
        // that were only closed are dropped here. The provider itself says
        // which is which: `merged_at` is set exactly on a merged one.
        let merged_only = state == PrState::Merged && self.target.provider == PrProvider::Github;

        let mut items: Vec<Value> = Vec::new();
        for page in 1..=MAX_LIST_PAGES {
            let mut req = ProviderRequest::new(&self.target, Method::Get, path.clone())
                .query("state", filter)
                .query("per_page", PAGE_SIZE.to_string());
            if page > 1 {
                req = req.query("page", page.to_string());
            }
            let resp = self.send(req).await?;
            if !resp.is_success() {
                return Err(self.provider_error("list the pull requests", &resp));
            }
            let Value::Array(page_items) = resp.json()? else {
                return Err(ToolError::internal(format!(
                    "{}: host '{}' answered a pull request list that is not a JSON array",
                    self.tool, self.target.host
                )));
            };
            let fetched = page_items.len();
            items.extend(page_items.into_iter().filter(|raw| !merged_only || is_merged(raw)).map(
                // The raw payload is moved into the model and serialized once;
                // the list item is then derived from that one object by
                // dropping the keys a list response cannot fill.
                |raw| list_item(self.normalize(raw, &PrSignals::unknown())),
            ));
            if fetched < PAGE_SIZE {
                break;
            }
        }
        let count = items.len();
        Ok(json!({"pull_requests": items, "count": count}))
    }

    /// FR-NEW-307: the full normalized model, enriched with the review and the
    /// check state, which live on other endpoints.
    ///
    /// GitHub costs three calls (the pull request, its reviews, the check runs
    /// of its head sha) and GitLab two, because GitLab reports its pipeline on
    /// the merge request payload itself and needs no third round trip.
    ///
    /// FR-NEW-308 is the rule that shapes this method: every sub-call failure is
    /// returned with `?`, so no path exists that produces a model with
    /// `review_state` or `checks_state` quietly set to `none`. Reporting "no
    /// failing checks" when the truth is "we could not find out" would make a
    /// caller merge on it.
    async fn get(&self, number: u64) -> Result<Value> {
        let resp = self
            .send(ProviderRequest::new(&self.target, Method::Get, self.pr_path(number)))
            .await?;
        if !resp.is_success() {
            // FR-NEW-313: the refusal names the number AND the repository, so a
            // caller who mistyped either one can see which.
            return Err(self.provider_error(&format!("read {}", self.subject(number)), &resp));
        }
        let raw = resp.json()?;
        let signals = match self.target.provider {
            PrProvider::Github => self.github_signals(number, &raw).await?,
            PrProvider::Gitlab => self.gitlab_signals(number, &raw).await?,
        };
        Ok(self.normalize(raw, &signals))
    }

    /// GitHub: reviews, then the check runs of the head sha taken from the pull
    /// request that was just read, which is the only place that sha is known.
    async fn github_signals(&self, number: u64, raw: &Value) -> Result<PrSignals> {
        let reviews = self
            .read_json(
                format!("/repos/{}/pulls/{number}/reviews", self.target.project_path()),
                "read the reviews",
            )
            .await?;
        let head_sha = raw.get("head").and_then(|h| h.get("sha")).and_then(Value::as_str);
        let checks = match head_sha.filter(|s| !s.is_empty()) {
            Some(sha) => {
                let runs = self
                    .read_json(
                        format!("/repos/{}/commits/{sha}/check-runs", self.target.project_path()),
                        "read the check runs",
                    )
                    .await?;
                github_checks_state(&runs)
            }
            // No head sha means the payload names no commit to ask about, so
            // there IS no check set to read: an observed absence, not a
            // swallowed failure.
            None => "none".to_string(),
        };
        Ok(PrSignals {
            review_state: Some(github_review_state(&reviews)),
            checks_state: Some(checks),
        })
    }

    /// GitLab: the approvals endpoint, plus the pipeline the merge request
    /// payload already carries.
    async fn gitlab_signals(&self, number: u64, raw: &Value) -> Result<PrSignals> {
        let approvals = self
            .read_json(
                format!("/projects/{}/merge_requests/{number}/approvals", self.encoded_project()),
                "read the approvals",
            )
            .await?;
        Ok(PrSignals {
            review_state: Some(gitlab_review_state(&approvals)),
            checks_state: Some(gitlab_checks_state(raw)),
        })
    }

    /// One GET whose answer must be JSON, with the provider's own refusal
    /// surfaced (FR-NEW-308). `what` names the endpoint in the error, because a
    /// caller must be able to tell WHICH sub-call failed.
    async fn read_json(&self, path: String, what: &str) -> Result<Value> {
        let resp = self.send(ProviderRequest::new(&self.target, Method::Get, path)).await?;
        if !resp.is_success() {
            return Err(self.provider_error(what, &resp));
        }
        resp.json()
    }

    /// FR-NEW-309: the unified diff, bounded by `git.max_pr_diff_mb`.
    ///
    /// GitHub serves the diff itself under a media type, so the cap is handed to
    /// the transport and the download stops at it. GitLab serves per file hunks
    /// as JSON, so the diff is assembled here and the assembly stops at the same
    /// cap; in both cases the bytes past the cap are never held.
    async fn diff(&self, number: u64, cap_mb: usize) -> Result<Value> {
        let cap = cap_mb.saturating_mul(1024 * 1024);
        let (diff, truncated) = match self.target.provider {
            PrProvider::Github => self.github_diff(number, cap).await?,
            PrProvider::Gitlab => self.gitlab_diff(number, cap).await?,
        };
        let bytes = diff.len();
        let mut out = json!({
            "pr_number": number,
            "diff": diff,
            "bytes": bytes,
            "truncated": truncated,
        });
        if truncated {
            // The remedy is named, because a truncated diff is not a reviewable
            // one and the caller needs to know where the rest is.
            out["note"] = json!(format!(
                "diff truncated at {cap_mb} MiB; fetch the branch with git.remote_fetch for the \
                 full change"
            ));
        }
        Ok(out)
    }

    async fn github_diff(&self, number: u64, cap: usize) -> Result<(String, bool)> {
        let req = ProviderRequest::new(
            &self.target,
            Method::Get,
            format!("/repos/{}/pulls/{number}", self.target.project_path()),
        )
        .accept(GITHUB_DIFF_MEDIA_TYPE)
        .truncate_at(cap);
        let resp = self.send(req).await?;
        if !resp.is_success() {
            return Err(self.provider_error("read the pull request diff", &resp));
        }
        // A 200 that is not a diff is the signature of a proxy or a login page
        // answering for the provider. The content type is named and the body is
        // NOT quoted: dumping an HTML page into a tool error helps nobody.
        // Parameters are dropped here as well as in the transport, so the media
        // type is compared and named the same way whatever produced it.
        if let Some(ct) = resp
            .content_type
            .as_deref()
            .map(|ct| ct.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
            .filter(|ct| !is_diff_content_type(ct))
        {
            return Err(ToolError::internal(format!(
                "{}: expected a unified diff from {}, got content-type '{ct}'",
                self.tool, self.target.host
            )));
        }
        Ok((resp.text(), resp.truncated))
    }

    /// GitLab answers each file's hunks with no `diff --git`, mode or `---`/`+++`
    /// header, so those are synthesized: without them the result is not a valid
    /// unified diff and no patch tool would read it.
    async fn gitlab_diff(&self, number: u64, cap: usize) -> Result<(String, bool)> {
        let payload = self
            .read_json(
                format!("/projects/{}/merge_requests/{number}/changes", self.encoded_project()),
                "read the merge request changes",
            )
            .await?;
        let changes = payload.get("changes").and_then(Value::as_array).map_or(&[][..], |a| a);
        let mut out = String::new();
        for change in changes {
            // Checked per file AND inside the appender, so the assembled string
            // never grows past the cap even by one large file.
            if out.len() >= cap {
                return Ok((out, true));
            }
            if !append_gitlab_change(&mut out, change, cap) {
                return Ok((out, true));
            }
        }
        Ok((out, false))
    }

    /// FR-NEW-310: ask the provider to merge, with the chosen strategy mapped
    /// to that provider's own vocabulary.
    ///
    /// The pull request is read first, on both providers. That read is what
    /// FR-NEW-311 needs to tell "already merged" and "closed" apart from every
    /// other refusal: both providers answer a merge on a merged pull request
    /// with the same opaque 405 as a failing required check, and a caller told
    /// "not mergeable" about work that already landed would go and redo it.
    async fn merge(
        &self,
        number: u64,
        strategy: MergeStrategy,
        title: Option<&str>,
        message: Option<&str>,
    ) -> Result<Value> {
        let current =
            self.read_json(self.pr_path(number), &format!("read {}", self.subject(number))).await?;
        // The state is read through the one normalizer, so the two providers'
        // spellings are compared in exactly one place.
        let state = self.normalize(current.clone(), &PrSignals::unknown());
        match state.get("state").and_then(Value::as_str).unwrap_or_default() {
            "merged" => {
                return Err(ToolError::invalid_argument(format!(
                    "{}: pull request {number} is already merged on host '{}'; nothing to merge",
                    self.tool, self.target.host
                )));
            }
            "closed" => {
                return Err(ToolError::invalid_argument(format!(
                    "{}: pull request {number} is closed on host '{}'; reopen it before merging",
                    self.tool, self.target.host
                )));
            }
            _ => {}
        }
        let merged = match self.target.provider {
            PrProvider::Github => {
                self.github_merge(number, strategy, title, message, current).await
            }
            PrProvider::Gitlab => self.gitlab_merge(number, strategy, title, message).await,
        }?;
        let mut out = merged;
        // FR-NEW-310: nothing local moved, and the answer says so. Silently
        // fetching on the caller's behalf would hide a network operation they
        // did not ask for.
        out["note"] = json!(format!(
            "merged on host '{}' with strategy '{}'; no local ref and no remote-tracking ref was \
             updated, run git.remote_fetch to observe the merge locally",
            self.target.host,
            strategy.label()
        ));
        Ok(out)
    }

    /// GitHub merges in one call: the strategy is the `merge_method` and a
    /// rebase needs no separate endpoint.
    ///
    /// The answer is a merge result (`sha`, `merged`, `message`), not a pull
    /// request, so it is folded into the payload the pre-read already returned:
    /// that keeps the normalized model complete AND puts the merge sha in
    /// `raw`. Both halves are the provider's own bytes.
    async fn github_merge(
        &self,
        number: u64,
        strategy: MergeStrategy,
        title: Option<&str>,
        message: Option<&str>,
        current: Value,
    ) -> Result<Value> {
        let mut payload = json!({"merge_method": strategy.github()});
        if let Some(t) = title {
            payload["commit_title"] = json!(t);
        }
        if let Some(m) = message {
            payload["commit_message"] = json!(m);
        }
        let req = ProviderRequest::new(
            &self.target,
            Method::Put,
            format!("/repos/{}/pulls/{number}/merge", self.target.project_path()),
        )
        .body(payload);
        let resp = self.send(req).await?;
        if !resp.is_success() {
            return Err(self.merge_error(strategy, &resp));
        }
        let mut raw = current;
        if let (Some(target), Some(result)) = (raw.as_object_mut(), resp.json()?.as_object()) {
            for (k, v) in result {
                target.insert(k.clone(), v.clone());
            }
        }
        Ok(self.normalize(raw, &PrSignals::unknown()))
    }

    /// GitLab expresses the same three intents differently: `squash` is a flag
    /// on the merge call, and a rebase is a separate, asynchronous endpoint that
    /// must complete before the merge is asked for.
    async fn gitlab_merge(
        &self,
        number: u64,
        strategy: MergeStrategy,
        title: Option<&str>,
        message: Option<&str>,
    ) -> Result<Value> {
        if strategy == MergeStrategy::Rebase {
            self.gitlab_rebase(number, strategy).await?;
        }
        let squash = strategy == MergeStrategy::Squash;
        let mut payload = json!({"squash": squash});
        // GitLab has no commit title field: the title and the body are one
        // message, and which key carries it depends on the strategy.
        if let Some(text) = commit_text(title, message) {
            let key = if squash { "squash_commit_message" } else { "merge_commit_message" };
            payload[key] = json!(text);
        }
        let req = ProviderRequest::new(
            &self.target,
            Method::Put,
            format!("/projects/{}/merge_requests/{number}/merge", self.encoded_project()),
        )
        .body(payload);
        let resp = self.send(req).await?;
        if !resp.is_success() {
            return Err(self.merge_error(strategy, &resp));
        }
        Ok(self.normalize(resp.json()?, &PrSignals::unknown()))
    }

    /// GitLab's rebase is accepted and then runs in the background, so it is
    /// polled until the provider says it finished. A rebase that failed reports
    /// `merge_error`, and that sentence is surfaced rather than followed by a
    /// merge that would refuse anyway.
    async fn gitlab_rebase(&self, number: u64, strategy: MergeStrategy) -> Result<()> {
        let path = format!("/projects/{}/merge_requests/{number}/rebase", self.encoded_project());
        let resp = self.send(ProviderRequest::new(&self.target, Method::Post, path)).await?;
        if !resp.is_success() {
            return Err(self.merge_error(strategy, &resp));
        }
        for attempt in 0..MAX_REBASE_POLLS {
            if attempt > 0 {
                tokio::time::sleep(REBASE_POLL_INTERVAL).await;
            }
            let req = ProviderRequest::new(&self.target, Method::Get, self.pr_path(number))
                .query("include_rebase_in_progress", "true");
            let resp = self.send(req).await?;
            if !resp.is_success() {
                return Err(self.provider_error("poll the rebase", &resp));
            }
            let state = resp.json()?;
            if let Some(err) = state.get("merge_error").and_then(Value::as_str) {
                return Err(ToolError::invalid_argument(format!(
                    "{}: host '{}' refused the rebase of pull request {number}: {err}",
                    self.tool, self.target.host
                )));
            }
            // An absent flag means the provider is not reporting a rebase any
            // more, which is how a finished one reads.
            if !state.get("rebase_in_progress").and_then(Value::as_bool).unwrap_or(false) {
                return Ok(());
            }
        }
        Err(ToolError::internal(format!(
            "{}: the rebase of pull request {number} on host '{}' was still running after {}s; \
             nothing was merged",
            self.tool,
            self.target.host,
            MAX_REBASE_POLLS as u64 * REBASE_POLL_INTERVAL.as_secs()
        )))
    }

    /// A merge refusal, named by the strategy that was refused: a repository
    /// with merge commits disabled answers the same 405 as a failing check, and
    /// the caller must see which of the two it is looking at (FR-NEW-311).
    fn merge_error(&self, strategy: MergeStrategy, resp: &ProviderResponse) -> ToolError {
        self.provider_error(
            &format!("merge the pull request with strategy '{}'", strategy.label()),
            resp,
        )
    }

    /// FR-NEW-312: submit the verdict, then report the review state it leaves
    /// behind.
    ///
    /// The result is NOT the normalized pull request: a review answer carries no
    /// pull request payload, and filling twenty one keys from it would put zeros
    /// and empty strings where the provider said nothing.
    async fn review(&self, number: u64, verdict: Verdict, body: &str) -> Result<Value> {
        let (review_state, raw) = match self.target.provider {
            PrProvider::Github => self.github_review(number, verdict, body).await?,
            PrProvider::Gitlab => self.gitlab_review(number, verdict, body).await?,
        };
        Ok(json!({
            "pr_number": number,
            "provider": self.target.provider.label(),
            "host": self.target.host,
            "verdict": verdict.label(),
            "review_state": review_state,
            "raw": raw,
        }))
    }

    /// GitHub reviews in exactly one call: the verdict is the review `event`.
    ///
    /// The state is read back from the provider's own answer rather than assumed
    /// from the verdict, and mapped through the same [`github_review_state`] the
    /// reviews endpoint goes through, so one mapper owns that vocabulary.
    async fn github_review(
        &self,
        number: u64,
        verdict: Verdict,
        body: &str,
    ) -> Result<(String, Value)> {
        let mut payload = json!({"event": verdict.github()});
        // No `body` key at all when none was supplied: an empty one would post
        // an empty comment alongside the verdict.
        if !body.is_empty() {
            payload["body"] = json!(body);
        }
        let req = ProviderRequest::new(
            &self.target,
            Method::Post,
            format!("/repos/{}/pulls/{number}/reviews", self.target.project_path()),
        )
        .body(payload);
        let resp = self.send(req).await?;
        if !resp.is_success() {
            return Err(self.review_error(number, &resp));
        }
        let raw = resp.json()?;
        let state = match raw.get("state").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            Some(s) => github_review_state(&json!([{"state": s}])),
            // An answer naming no state is not a reason to invent one: the
            // verdict that was accepted says what the state became.
            None => verdict.review_state().to_string(),
        };
        Ok((state, raw))
    }

    /// GitLab has no review verdict, so the three intents reach two genuinely
    /// different endpoints: `approve` is an approval, the other two are a note.
    ///
    /// Requesting changes withdraws any standing approval first, because GitLab
    /// would otherwise keep counting this reviewer as an approver while their
    /// note asks for work.
    async fn gitlab_review(
        &self,
        number: u64,
        verdict: Verdict,
        body: &str,
    ) -> Result<(String, Value)> {
        if verdict == Verdict::Approve {
            let req = ProviderRequest::new(
                &self.target,
                Method::Post,
                format!("/projects/{}/merge_requests/{number}/approve", self.encoded_project()),
            )
            // An empty object, not an absent body: GitLab's approve takes no
            // field, and stating it keeps the request self describing.
            .body(json!({}));
            let resp = self.send(req).await?;
            if !resp.is_success() {
                return Err(self.review_error(number, &resp));
            }
            let raw = resp.json()?;
            // The approve answer carries no approval information, so the
            // resulting state is read from the approvals endpoint rather than
            // assumed: a second approver may still be required.
            let approvals = self
                .read_json(
                    format!(
                        "/projects/{}/merge_requests/{number}/approvals",
                        self.encoded_project()
                    ),
                    "read the approvals",
                )
                .await?;
            return Ok((gitlab_review_state(&approvals), raw));
        }
        if verdict == Verdict::RequestChanges {
            let req = ProviderRequest::new(
                &self.target,
                Method::Post,
                format!("/projects/{}/merge_requests/{number}/unapprove", self.encoded_project()),
            );
            let resp = self.send(req).await?;
            // A 404 here means there was no approval of ours to withdraw, which
            // is the normal case and not a failure: the note still goes out.
            if !resp.is_success() && resp.status != 404 {
                return Err(self.review_error(number, &resp));
            }
        }
        let req = ProviderRequest::new(
            &self.target,
            Method::Post,
            format!("/projects/{}/merge_requests/{number}/notes", self.encoded_project()),
        )
        .body(json!({"body": body}));
        let resp = self.send(req).await?;
        if !resp.is_success() {
            return Err(self.review_error(number, &resp));
        }
        // GitLab records no verdict on a note, so the state is the one the
        // verdict defines. This IS the normalization: the caller asked for
        // changes and the answer says so on both providers.
        Ok((verdict.review_state().to_string(), resp.json()?))
    }

    /// A review refusal, naming the pull request it was refused on (FR-NEW-313).
    fn review_error(&self, number: u64, resp: &ProviderResponse) -> ToolError {
        self.provider_error(&format!("submit the review on {}", self.subject(number)), resp)
    }

    /// How a refusal names the thing it is about: each provider's own noun, the
    /// number, and the repository.
    fn subject(&self, number: u64) -> String {
        let noun = match self.target.provider {
            PrProvider::Github => "pull request",
            PrProvider::Gitlab => "merge request",
        };
        format!("{noun} {number} of '{}'", self.target.project_path())
    }

    /// The single pull request route, the one both a read and a merge pre-read
    /// address.
    fn pr_path(&self, number: u64) -> String {
        match self.target.provider {
            PrProvider::Github => format!("/repos/{}/pulls/{number}", self.target.project_path()),
            PrProvider::Gitlab => {
                format!("/projects/{}/merge_requests/{number}", self.encoded_project())
            }
        }
    }

    /// `owner/repo` percent encoded, which is how GitLab addresses a project in
    /// a path segment.
    fn encoded_project(&self) -> String {
        self.target.project_path().replace('/', "%2F")
    }

    /// One place maps a payload, so no call site can assemble the keys itself.
    fn normalize(&self, raw: Value, signals: &PrSignals) -> Value {
        match self.target.provider {
            PrProvider::Github => PullRequest::from_github(&self.target.host, raw, signals),
            PrProvider::Gitlab => PullRequest::from_gitlab(&self.target.host, raw, signals),
        }
        .to_value()
    }

    /// The provider's own refusal, kept faithful: its status and its message,
    /// mapped onto the closest `ERR_*` so a caller can branch. The body was
    /// already scrubbed of the credential by the transport.
    fn provider_error(&self, what: &str, resp: &ProviderResponse) -> ToolError {
        let detail = resp
            .json()
            .ok()
            .map(|v| provider_detail(&v))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| resp.text());
        let detail: String = detail.chars().take(MAX_PROVIDER_DETAIL).collect();
        let mut message = format!(
            "{}: {} on host '{}' failed with HTTP {}: {detail}",
            self.tool, what, self.target.host, resp.status
        );
        if resp.status == 401 {
            // A provider 401 means the credential is no longer accepted, and a
            // caller who is not told the remedy will simply retry it.
            message.push_str(&format!(
                "; re-authenticate with git.auth or git.token_set for host '{}'",
                self.target.host
            ));
        }
        match resp.status {
            401 => ToolError::unauthenticated(message),
            403 => ToolError::forbidden(message),
            404 => ToolError::not_found(message),
            // 409 duplicate, 422 unprocessable: a caller fixable request. 405
            // is one too on this surface: both providers answer a refused merge
            // with it, and it names a condition the caller can act on rather
            // than a server fault.
            400 | 405 | 409 | 422 => ToolError::invalid_argument(message),
            _ => ToolError::internal(message),
        }
    }

    async fn send(&self, req: ProviderRequest) -> Result<ProviderResponse> {
        self.client.send(&req, &self.cred).await
    }
}

/// How much of a provider message is quoted back. A bound, not a judgement: a
/// provider that answers with a whole HTML error page must not put it in a tool
/// error.
const MAX_PROVIDER_DETAIL: usize = 500;

/// Items asked for per page: the maximum both providers accept, so a listing
/// costs the fewest round trips.
const PAGE_SIZE: usize = 100;

/// How long a GitLab rebase is waited for, and how often it is asked about.
/// A bound, not a judgement: without it a rebase that never finishes would hold
/// the request forever.
const MAX_REBASE_POLLS: usize = 60;
const REBASE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// How many pages a listing walks. A bound, not a judgement: without it a
/// provider that keeps answering full pages would hold the request for as long
/// as it cared to.
const MAX_LIST_PAGES: usize = 20;

/// The five keys only a single pull request read can fill. A list response
/// carries neither the counts nor mergeability, and reporting zeros there would
/// be an invented answer, so they are dropped from a list item (FR-NEW-306).
const LIST_OMITTED_KEYS: [&str; 5] =
    ["commits", "changed_files", "additions", "deletions", "mergeable"];

/// Derive a list item from the normalized object, keeping the remaining
/// sixteen keys in their frozen relative order. Derived, never hand built: a
/// second struct is exactly the drift this surface kept re-discovering.
fn list_item(mut normalized: Value) -> Value {
    if let Some(map) = normalized.as_object_mut() {
        for key in LIST_OMITTED_KEYS {
            // `shift_remove`, never `remove`: serde_json is built with
            // `preserve_order`, whose `remove` swaps the last entry into the
            // hole and would reorder the key set this story freezes.
            map.shift_remove(key);
        }
    }
    normalized
}

/// What `git.pr_diff` asks GitHub for: the pull request route serves the unified
/// diff itself under this media type, so no second endpoint is involved.
const GITHUB_DIFF_MEDIA_TYPE: &str = "application/vnd.github.v3.diff";

/// The content types a unified diff arrives under. A provider that answers
/// anything else under a 200 did not serve a diff, whatever the status says.
const DIFF_CONTENT_TYPES: [&str; 4] =
    ["text/plain", "text/x-diff", "text/x-patch", "application/x-patch"];

fn is_diff_content_type(ct: &str) -> bool {
    DIFF_CONTENT_TYPES.contains(&ct) || ct.starts_with("application/vnd.github")
}

/// FR-NEW-307: GitHub's review state, from the reviews endpoint.
///
/// Only the LAST review of each reviewer counts, which is how GitHub itself
/// computes mergeability: an earlier "changes requested" that the same reviewer
/// later approved must not keep blocking. `COMMENTED` and `DISMISSED` carry no
/// verdict, so a pull request holding only those is `review_required`: someone
/// looked, nobody decided.
fn github_review_state(reviews: &Value) -> String {
    let Some(items) = reviews.as_array() else {
        return "none".to_string();
    };
    let mut latest: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    let mut commented = false;
    for review in items {
        let state = review.get("state").and_then(Value::as_str).unwrap_or_default();
        let who =
            review.get("user").and_then(|u| u.get("login")).and_then(Value::as_str).unwrap_or("");
        match state.to_ascii_uppercase().as_str() {
            "APPROVED" => {
                latest.insert(who, "approved");
            }
            "CHANGES_REQUESTED" => {
                latest.insert(who, "changes_requested");
            }
            // A dismissal withdraws that reviewer's verdict; a comment never was
            // one. Both leave the reviewer without a decision.
            "DISMISSED" => {
                latest.remove(who);
            }
            _ => commented = true,
        }
    }
    if latest.values().any(|v| *v == "changes_requested") {
        return "changes_requested".to_string();
    }
    if latest.values().any(|v| *v == "approved") {
        return "approved".to_string();
    }
    if commented { "review_required".to_string() } else { "none".to_string() }
}

/// FR-NEW-307: GitHub's check state, from the check runs of the head commit.
///
/// A run that has not completed makes the whole set `pending`, because the
/// answer is not in yet; `neutral` and `skipped` are successes, which is how
/// GitHub's own required checks treat them.
fn github_checks_state(runs: &Value) -> String {
    let items = runs.get("check_runs").and_then(Value::as_array).map_or(&[][..], |a| a);
    if items.is_empty() {
        return "none".to_string();
    }
    let mut pending = false;
    for run in items {
        let status = run.get("status").and_then(Value::as_str).unwrap_or_default();
        if status != "completed" {
            pending = true;
            continue;
        }
        match run.get("conclusion").and_then(Value::as_str).unwrap_or_default() {
            "success" | "neutral" | "skipped" => {}
            // `action_required` blocks a merge exactly as a failure does.
            "failure" | "timed_out" | "cancelled" | "stale" | "action_required" => {
                return "failure".to_string();
            }
            // A completed run with no conclusion yet: the answer is not in.
            _ => pending = true,
        }
    }
    if pending { "pending".to_string() } else { "success".to_string() }
}

/// FR-NEW-307: GitLab's review state, from the approvals endpoint. `approved`
/// is the provider's own verdict; an outstanding requirement is
/// `review_required`; GitLab has no "changes requested" verdict at all.
fn gitlab_review_state(approvals: &Value) -> String {
    if approvals.get("approved").and_then(Value::as_bool).unwrap_or(false) {
        return "approved".to_string();
    }
    let approved_by = approvals.get("approved_by").and_then(Value::as_array).map_or(0, |a| a.len());
    if approved_by > 0 {
        return "approved".to_string();
    }
    let left = approvals.get("approvals_left").and_then(Value::as_u64).unwrap_or(0);
    let required = approvals.get("approvals_required").and_then(Value::as_u64).unwrap_or(0);
    if left > 0 || required > 0 { "review_required".to_string() } else { "none".to_string() }
}

/// FR-NEW-307: GitLab's check state, from the pipeline the merge request payload
/// already carries, which is why GitLab costs one call fewer than GitHub.
fn gitlab_checks_state(raw: &Value) -> String {
    let status = raw
        .get("head_pipeline")
        .and_then(|p| p.get("status"))
        .or_else(|| raw.get("pipeline").and_then(|p| p.get("status")))
        .and_then(Value::as_str);
    match status {
        Some("success") => "success".to_string(),
        Some("failed" | "canceled" | "cancelled") => "failure".to_string(),
        // No pipeline at all, or one that never ran: nothing was checked.
        None | Some("" | "skipped") => "none".to_string(),
        _ => "pending".to_string(),
    }
}

/// Synthesize one file's unified diff header, then its hunks, stopping at `cap`.
/// Returns false when the cap was reached, so the caller marks the answer
/// truncated.
///
/// The `---`/`+++` pair is emitted only for a body that starts with a hunk:
/// GitLab reports a binary change as a one line sentence, and putting file
/// headers around that would invent a hunk that does not exist.
fn append_gitlab_change(out: &mut String, change: &Value, cap: usize) -> bool {
    let field = |k: &str| change.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |k: &str| change.get(k).and_then(Value::as_bool).unwrap_or(false);
    let old_path = field("old_path");
    let new_path = field("new_path");
    let body = field("diff");
    let (new_file, deleted, renamed) =
        (flag("new_file"), flag("deleted_file"), flag("renamed_file"));

    let mut header = format!("diff --git a/{old_path} b/{new_path}\n");
    if new_file {
        header.push_str("new file mode 100644\n");
    } else if deleted {
        header.push_str("deleted file mode 100644\n");
    } else if renamed {
        header.push_str(&format!("rename from {old_path}\nrename to {new_path}\n"));
    }
    if body.starts_with("@@") {
        let from =
            if new_file { "--- /dev/null\n".to_string() } else { format!("--- a/{old_path}\n") };
        let to =
            if deleted { "+++ /dev/null\n".to_string() } else { format!("+++ b/{new_path}\n") };
        header.push_str(&from);
        header.push_str(&to);
    }
    push_bounded(out, &header, cap) && push_bounded(out, &body, cap)
}

/// Append as much of `text` as the cap allows, on a char boundary. Returns false
/// when it did not all fit.
fn push_bounded(out: &mut String, text: &str, cap: usize) -> bool {
    let room = cap.saturating_sub(out.len());
    if text.len() <= room {
        out.push_str(text);
        return true;
    }
    let mut end = room;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&text[..end]);
    false
}

/// The one commit message GitLab accepts, built from the two fields GitHub
/// keeps apart. Returns `None` when the caller supplied neither, so the
/// provider's own default stands.
fn commit_text(title: Option<&str>, message: Option<&str>) -> Option<String> {
    match (title, message) {
        (Some(t), Some(m)) => Some(format!("{t}\n\n{m}")),
        (Some(t), None) => Some(t.to_string()),
        (None, Some(m)) => Some(m.to_string()),
        (None, None) => None,
    }
}

/// The sentence a provider refusal is worth quoting back.
///
/// GitHub splits it: `message` says "Unprocessable Entity" and the actionable
/// reason sits in `errors`, whose items are sometimes strings and sometimes
/// objects. Both halves are kept, because either one alone loses the answer.
fn provider_detail(v: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(head) = ["message", "error", "error_description"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
    {
        parts.push(head.to_string());
    }
    if let Some(items) = v.get("errors").and_then(Value::as_array) {
        parts.extend(items.iter().filter_map(|e| match e {
            Value::String(s) => Some(s.clone()),
            _ => e.get("message").and_then(Value::as_str).map(str::to_string),
        }));
    }
    parts.join(": ")
}

/// GitHub marks a merged pull request with a non null `merged_at`.
fn is_merged(raw: &Value) -> bool {
    raw.get("merged_at").is_some_and(|v| !v.is_null())
}

#[cfg(test)]
mod tests;
