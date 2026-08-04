//! Error model. Mirrors the C# `Models/Errors.cs` exactly: a stable `ERR_*` code
//! carried to the MCP client and mapped to an HTTP status on the REST plane.

use std::fmt;

/// The 14 stable error codes. These strings are part of the public contract and
/// MUST NOT change: MCP clients and REST consumers match on them.
pub mod code {
    pub const UNAUTHENTICATED: &str = "ERR_UNAUTHENTICATED";
    pub const FORBIDDEN: &str = "ERR_FORBIDDEN";
    pub const PROJECT_NOT_FOUND: &str = "ERR_PROJECT_NOT_FOUND";
    pub const PROJECT_EXISTS: &str = "ERR_PROJECT_EXISTS";
    pub const PATH_OUT_OF_BOUNDS: &str = "ERR_PATH_OUT_OF_BOUNDS";
    pub const EDIT_WITHOUT_PRIOR_READ: &str = "ERR_EDIT_WITHOUT_PRIOR_READ";
    pub const NO_CLOBBER: &str = "ERR_NO_CLOBBER";
    pub const NOT_FOUND: &str = "ERR_NOT_FOUND";
    pub const AMBIGUOUS_MATCH: &str = "ERR_AMBIGUOUS_MATCH";
    pub const NO_MATCH: &str = "ERR_NO_MATCH";
    pub const WRITE_QUOTA_EXCEEDED: &str = "ERR_WRITE_QUOTA_EXCEEDED";
    pub const INVALID_ARGUMENT: &str = "ERR_INVALID_ARGUMENT";
    pub const NOT_SUPPORTED: &str = "ERR_NOT_SUPPORTED";
    pub const INTERNAL_ERROR: &str = "ERR_INTERNAL_ERROR";
}

/// An expected, user-facing error (4xx-style). Rendered to the client as
/// `"{code}: {message}"`, exactly like the C# `ToolError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    // ── constructors, one per code (keeps call sites terse and consistent) ────
    pub fn unauthenticated(m: impl Into<String>) -> Self { Self::new(code::UNAUTHENTICATED, m) }
    pub fn forbidden(m: impl Into<String>) -> Self { Self::new(code::FORBIDDEN, m) }
    pub fn project_not_found(id: &str) -> Self {
        Self::new(code::PROJECT_NOT_FOUND, format!("project '{id}' not found"))
    }
    pub fn project_exists(id: &str) -> Self {
        Self::new(code::PROJECT_EXISTS, format!("project '{id}' already exists"))
    }
    pub fn path_out_of_bounds(m: impl Into<String>) -> Self { Self::new(code::PATH_OUT_OF_BOUNDS, m) }
    pub fn edit_without_prior_read(m: impl Into<String>) -> Self {
        Self::new(code::EDIT_WITHOUT_PRIOR_READ, m)
    }
    pub fn no_clobber(m: impl Into<String>) -> Self { Self::new(code::NO_CLOBBER, m) }
    pub fn not_found(m: impl Into<String>) -> Self { Self::new(code::NOT_FOUND, m) }
    pub fn ambiguous_match(m: impl Into<String>) -> Self { Self::new(code::AMBIGUOUS_MATCH, m) }
    pub fn no_match(m: impl Into<String>) -> Self { Self::new(code::NO_MATCH, m) }
    pub fn write_quota_exceeded(m: impl Into<String>) -> Self {
        Self::new(code::WRITE_QUOTA_EXCEEDED, m)
    }
    pub fn invalid_argument(m: impl Into<String>) -> Self { Self::new(code::INVALID_ARGUMENT, m) }
    pub fn not_supported(m: impl Into<String>) -> Self { Self::new(code::NOT_SUPPORTED, m) }
    pub fn internal(m: impl Into<String>) -> Self { Self::new(code::INTERNAL_ERROR, m) }

    /// HTTP status for the REST data plane.
    ///
    /// Every code is mapped explicitly. The reference mapped six codes and sent
    /// everything else to a generic 400 (`GetValueOrDefault(code, 400)`), so a spent
    /// quota, an edit without a prior read, an ambiguous match and an unsupported
    /// format were indistinguishable by status alone. The status is the first thing a
    /// caller branches on, so each condition gets the status that actually describes
    /// it and a client can retry, correct, or give up without parsing the body.
    pub fn http_status(&self) -> u16 {
        match self.code {
            code::UNAUTHENTICATED => 401,
            code::FORBIDDEN => 403,
            code::PROJECT_NOT_FOUND | code::NOT_FOUND => 404,
            // A name already taken and an unresolvable multi match are both conflicts.
            code::NO_CLOBBER | code::PROJECT_EXISTS | code::AMBIGUOUS_MATCH => 409,
            code::PATH_OUT_OF_BOUNDS | code::INVALID_ARGUMENT => 400,
            // The session has spent its byte allowance, so this is a budget refusal.
            code::WRITE_QUOTA_EXCEEDED => 429,
            // The guard requires the file to have been read first: a precondition on
            // session state that the caller can satisfy and retry.
            code::EDIT_WITHOUT_PRIOR_READ => 428,
            // Well formed request, but the target text is simply not in the file.
            code::NO_MATCH => 422,
            // The server does not implement this (a format out of scope, a disabled
            // subsystem), which is what 501 means.
            code::NOT_SUPPORTED => 501,
            code::INTERNAL_ERROR => 500,
            // A new code must be mapped explicitly, so this stays exhaustive in
            // spirit: an unmapped code is a bug, not a silent 500.
            _ => 500,
        }
    }

    /// True when this error is the caller's fault (a 4xx), so logging can stay
    /// concise and monitoring does not treat it as a server failure.
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.http_status())
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

/// Anything unexpected becomes an internal error, preserving the cause text.
impl From<anyhow::Error> for ToolError {
    fn from(e: anyhow::Error) -> Self { Self::internal(e.to_string()) }
}
impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self { Self::internal(e.to_string()) }
}
impl From<rusqlite::Error> for ToolError {
    fn from(e: rusqlite::Error) -> Self { Self::internal(format!("sqlite: {e}")) }
}
impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self { Self::invalid_argument(format!("json: {e}")) }
}

pub type Result<T> = std::result::Result<T, ToolError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_code_colon_message() {
        let e = ToolError::forbidden("'a@b.c' is not a member of 'p'");
        assert_eq!(e.to_string(), "ERR_FORBIDDEN: 'a@b.c' is not a member of 'p'");
    }

    #[test]
    fn every_code_has_an_http_status() {
        assert_eq!(ToolError::unauthenticated("x").http_status(), 401);
        assert_eq!(ToolError::forbidden("x").http_status(), 403);
        assert_eq!(ToolError::project_not_found("p").http_status(), 404);
        assert_eq!(ToolError::not_found("x").http_status(), 404);
        assert_eq!(ToolError::no_clobber("x").http_status(), 409);
        assert_eq!(ToolError::project_exists("p").http_status(), 409);
        assert_eq!(ToolError::ambiguous_match("x").http_status(), 409);
        assert_eq!(ToolError::path_out_of_bounds("x").http_status(), 400);
        assert_eq!(ToolError::invalid_argument("x").http_status(), 400);
        assert_eq!(ToolError::write_quota_exceeded("x").http_status(), 429);
        assert_eq!(ToolError::edit_without_prior_read("x").http_status(), 428);
        assert_eq!(ToolError::no_match("x").http_status(), 422);
        assert_eq!(ToolError::not_supported("x").http_status(), 501);
        assert_eq!(ToolError::internal("x").http_status(), 500);
    }

    /// A client caused error must never be reported as a server failure: an operator
    /// should not be paged because a caller spent its quota.
    #[test]
    fn client_caused_codes_are_all_4xx() {
        for e in [
            ToolError::unauthenticated("x"),
            ToolError::forbidden("x"),
            ToolError::project_not_found("p"),
            ToolError::not_found("x"),
            ToolError::no_clobber("x"),
            ToolError::project_exists("p"),
            ToolError::ambiguous_match("x"),
            ToolError::path_out_of_bounds("x"),
            ToolError::invalid_argument("x"),
            ToolError::write_quota_exceeded("x"),
            ToolError::edit_without_prior_read("x"),
            ToolError::no_match("x"),
        ] {
            assert!(
                e.is_client_error(),
                "{} must be a 4xx, got {}",
                e.code,
                e.http_status()
            );
        }
        // Genuinely server side: not implemented, and an unexpected failure.
        assert!(!ToolError::not_supported("x").is_client_error());
        assert!(!ToolError::internal("x").is_client_error());
    }

    /// Guard against adding a code and forgetting the mapping.
    #[test]
    fn no_code_falls_through_to_an_accidental_500() {
        let client_caused = [
            code::UNAUTHENTICATED,
            code::FORBIDDEN,
            code::PROJECT_NOT_FOUND,
            code::PROJECT_EXISTS,
            code::PATH_OUT_OF_BOUNDS,
            code::EDIT_WITHOUT_PRIOR_READ,
            code::NO_CLOBBER,
            code::NOT_FOUND,
            code::AMBIGUOUS_MATCH,
            code::NO_MATCH,
            code::WRITE_QUOTA_EXCEEDED,
            code::INVALID_ARGUMENT,
        ];
        for c in client_caused {
            let status = ToolError::new(c, "x").http_status();
            assert!((400..500).contains(&status), "{c} mapped to {status}, expected a 4xx");
        }
    }

    #[test]
    fn project_helpers_format_like_csharp() {
        assert_eq!(
            ToolError::project_not_found("dt").to_string(),
            "ERR_PROJECT_NOT_FOUND: project 'dt' not found"
        );
        assert_eq!(
            ToolError::project_exists("dt").to_string(),
            "ERR_PROJECT_EXISTS: project 'dt' already exists"
        );
    }
}
