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

    /// HTTP status for the REST data plane. Mirrors the C# `HttpForCode` map.
    pub fn http_status(&self) -> u16 {
        match self.code {
            code::UNAUTHENTICATED => 401,
            code::FORBIDDEN => 403,
            code::PROJECT_NOT_FOUND | code::NOT_FOUND => 404,
            code::NO_CLOBBER => 409,
            code::PATH_OUT_OF_BOUNDS | code::INVALID_ARGUMENT => 400,
            _ => 500,
        }
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
    fn http_status_mapping_matches_csharp() {
        assert_eq!(ToolError::unauthenticated("x").http_status(), 401);
        assert_eq!(ToolError::forbidden("x").http_status(), 403);
        assert_eq!(ToolError::project_not_found("p").http_status(), 404);
        assert_eq!(ToolError::not_found("x").http_status(), 404);
        assert_eq!(ToolError::no_clobber("x").http_status(), 409);
        assert_eq!(ToolError::path_out_of_bounds("x").http_status(), 400);
        assert_eq!(ToolError::invalid_argument("x").http_status(), 400);
        assert_eq!(ToolError::internal("x").http_status(), 500);
        // codes without an explicit HTTP mapping fall back to 500
        assert_eq!(ToolError::no_match("x").http_status(), 500);
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
