//! Which OAuth scopes the pull request surface needs, and whether the scope set
//! recorded with a stored token holds them (FR-NEW-332, FR-NEW-333, FR-NEW-334).
//!
//! The check is a string set comparison over data already loaded with the token,
//! so it costs neither a query nor a round trip. It runs before the provider is
//! contacted, mirroring the expired token rule in
//! [`crate::git::oauth::store::OAuthTokenStore::require_valid_credential`]:
//! failing at the provider would surface an opaque 403 with no remedy in it.
//!
//! One deliberate asymmetry (DEC-911): a scope set that is *known* to be
//! insufficient fails early, while an *empty or unknown* one is attempted and
//! the provider's own answer is surfaced. `git.token_set` seeds personal access
//! tokens whose scopes this server cannot enumerate, so refusing them up front
//! would make that tool useless for its main purpose.

use crate::errors::{Result, ToolError};

/// The half of the pull request surface an operation needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrAccess {
    /// `git.pr_list`, `git.pr_get`, `git.pr_diff`.
    Read,
    /// `git.pr_create`, `git.pr_review`, `git.pr_merge`.
    Write,
}

/// What the recorded scope set says about the pull request surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrCapability {
    Capable,
    /// The scope set is empty, or the host's provider has no scope vocabulary
    /// this server knows. Not a refusal: the operation is attempted.
    Unknown,
    /// Known and known to be short of what the surface needs. `missing` holds
    /// the scope to request, one entry, canonical spelling.
    Insufficient {
        missing: Vec<String>,
    },
}

impl PrCapability {
    /// `Some(true)`/`Some(false)` when the answer is known, `None` when it is
    /// not: `git.auth_status` reports the three states distinctly (FR-NEW-335).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Capable => Some(true),
            Self::Insufficient { .. } => Some(false),
            Self::Unknown => None,
        }
    }

    pub fn missing(&self) -> &[String] {
        match self {
            Self::Insufficient { missing } => missing,
            _ => &[],
        }
    }
}

/// The scopes that satisfy `access` on `provider`, the canonical one to request
/// when none is held, and how to name the requirement to a human.
///
/// `None` means this server does not know the provider's scope vocabulary, so it
/// has no basis on which to refuse.
fn requirement(
    provider: &str,
    access: PrAccess,
) -> Option<(&'static [&'static str], &'static str, &'static str)> {
    match provider {
        // GitHub's `repo` already covers the whole pull request surface, read
        // and write alike, so there is nothing to add for it.
        "github" => Some((&["repo"], "repo", "'repo'")),
        // GitLab documents `write_repository` as Git over HTTP access only,
        // granting no REST API access whatsoever, so the API scopes are the
        // only ones that count here. `api` subsumes `read_api`.
        "gitlab" => match access {
            PrAccess::Read => Some((&["api", "read_api"], "api", "'api' (or 'read_api')")),
            PrAccess::Write => Some((&["api"], "api", "'api'")),
        },
        _ => None,
    }
}

/// Judge a recorded scope set. Pure: no I/O, no allocation beyond the missing
/// scope name.
pub fn pr_capability(provider: &str, scopes: &[String], access: PrAccess) -> PrCapability {
    // An empty set is unrecorded, not narrow: see the module docs.
    if scopes.is_empty() {
        return PrCapability::Unknown;
    }
    match requirement(provider, access) {
        None => PrCapability::Unknown,
        Some((accepted, canonical, _)) => {
            if scopes.iter().any(|s| accepted.contains(&s.as_str())) {
                PrCapability::Capable
            } else {
                PrCapability::Insufficient { missing: vec![canonical.to_string()] }
            }
        }
    }
}

/// The gate every `git.pr_*` tool runs before touching the network. The message
/// names the missing scope, the host, and both tools that grant a replacement
/// (FR-NEW-333); it never echoes the token or the scopes held.
pub fn require_pr_scope(
    provider: &str,
    scopes: &[String],
    host: &str,
    access: PrAccess,
) -> Result<()> {
    // An empty set is unrecorded, not narrow, and an unknown vocabulary gives
    // this server no basis on which to refuse: both are attempted.
    let Some((accepted, _, names)) = requirement(provider, access).filter(|_| !scopes.is_empty())
    else {
        return Ok(());
    };
    if scopes.iter().any(|s| accepted.contains(&s.as_str())) {
        return Ok(());
    }
    Err(ToolError::forbidden(format!(
        "the token for host {host} is missing scope {names}; re-authenticate with git.auth to \
         grant it, or supply a token that holds it with git.token_set"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| i.to_string()).collect()
    }

    /// E2E-NEW-782: no wildcard interpretation and no "looks powerful"
    /// heuristic; the read message names `api` and its alternative.
    #[test]
    fn e2e_new_782_a_powerful_looking_scope_set_is_still_insufficient() {
        let scopes = s(&["everything", "super_admin", "*"]);
        assert_eq!(
            pr_capability("gitlab", &scopes, PrAccess::Read),
            PrCapability::Insufficient { missing: s(&["api"]) }
        );
        let e = require_pr_scope("gitlab", &scopes, "gitlab.com", PrAccess::Read).unwrap_err();
        assert_eq!(e.code, crate::errors::code::FORBIDDEN);
        assert!(e.message.contains("missing scope 'api' (or 'read_api')"), "{}", e.message);
    }

    /// E2E-NEW-784: `api` alone satisfies both halves of the surface, so no
    /// path may require `read_api` as well.
    #[test]
    fn e2e_new_784_api_alone_satisfies_read_and_write() {
        let scopes = s(&["api"]);
        for access in [PrAccess::Read, PrAccess::Write] {
            assert_eq!(pr_capability("gitlab", &scopes, access), PrCapability::Capable);
            require_pr_scope("gitlab", &scopes, "gitlab.com", access).unwrap();
        }
        // read_api reads but does not write.
        let ro = s(&["read_api"]);
        assert_eq!(pr_capability("gitlab", &ro, PrAccess::Read), PrCapability::Capable);
        assert_eq!(
            pr_capability("gitlab", &ro, PrAccess::Write),
            PrCapability::Insufficient { missing: s(&["api"]) }
        );
    }

    /// E2E-NEW-817: the pre-change GitLab scope pair is known and known to be
    /// insufficient, and the message carries all four actionable parts without
    /// echoing anything token shaped.
    #[test]
    fn e2e_new_817_the_old_scope_pair_is_refused_with_an_actionable_message() {
        let scopes = s(&["read_repository", "write_repository"]);
        let e = require_pr_scope("gitlab", &scopes, "gitlab.com", PrAccess::Read).unwrap_err();
        assert_eq!(e.code, crate::errors::code::FORBIDDEN);
        for part in ["api", "gitlab.com", "git.auth", "git.token_set"] {
            assert!(e.message.contains(part), "missing '{part}' in: {}", e.message);
        }
        assert!(!e.message.contains("glpat"), "{}", e.message);
    }

    /// E2E-NEW-928: the same message shape for every member of the family, on
    /// both halves of the surface, naming the host it was seeded for.
    #[test]
    fn e2e_new_928_the_scope_error_names_the_host_and_both_remedy_tools() {
        let scopes = s(&["read_repository"]);
        for access in [PrAccess::Read, PrAccess::Write] {
            let e = require_pr_scope("gitlab", &scopes, "gitlab.example.test", access).unwrap_err();
            assert_eq!(e.code, crate::errors::code::FORBIDDEN);
            for part in ["api", "gitlab.example.test", "git.auth", "git.token_set"] {
                assert!(e.message.contains(part), "missing '{part}' in: {}", e.message);
            }
            assert!(!e.message.contains("glpat"), "{}", e.message);
            assert!(
                !e.message.contains("read_repository"),
                "naming the missing scope is required, echoing the held ones is not: {}",
                e.message
            );
        }
    }

    /// E2E-NEW-781 / E2E-NEW-929: an empty scope set is unknown, not
    /// insufficient, on every provider, so the call is attempted (DEC-911).
    #[test]
    fn e2e_new_781_an_empty_scope_set_is_attempted_not_refused() {
        for provider in ["github", "gitlab", "generic"] {
            for access in [PrAccess::Read, PrAccess::Write] {
                assert_eq!(
                    pr_capability(provider, &[], access),
                    PrCapability::Unknown,
                    "{provider} with no recorded scopes"
                );
                require_pr_scope(provider, &[], "any.host.test", access).unwrap();
            }
        }
    }

    /// FR-NEW-334: a provider whose scope vocabulary is unknown is also given
    /// the benefit of the doubt, even when its scope set is non-empty.
    #[test]
    fn an_unknown_provider_vocabulary_is_never_a_refusal() {
        let scopes = s(&["whatever"]);
        assert_eq!(pr_capability("generic", &scopes, PrAccess::Write), PrCapability::Unknown);
        require_pr_scope("generic", &scopes, "git.acme.internal", PrAccess::Write).unwrap();
    }

    /// FR-NEW-332: GitHub's `repo` is sufficient and unchanged; anything else
    /// is named as missing `repo`, not `api`.
    #[test]
    fn github_requires_repo_on_both_halves() {
        assert_eq!(pr_capability("github", &s(&["repo"]), PrAccess::Write), PrCapability::Capable);
        let e = require_pr_scope("github", &s(&["read:user"]), "github.com", PrAccess::Read)
            .unwrap_err();
        assert!(e.message.contains("missing scope 'repo'"), "{}", e.message);
    }

    #[test]
    fn capability_reports_three_distinct_states() {
        assert_eq!(PrCapability::Capable.as_bool(), Some(true));
        assert_eq!(PrCapability::Unknown.as_bool(), None);
        assert_eq!(PrCapability::Insufficient { missing: s(&["api"]) }.as_bool(), Some(false));
        assert_eq!(PrCapability::Unknown.missing(), &[] as &[String]);
    }
}
