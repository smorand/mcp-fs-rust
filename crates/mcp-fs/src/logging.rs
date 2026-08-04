//! Tracing setup plus the "expected error" classification.
//!
//! Port of the intent of the C# `Logging/ExpectedErrorFilter.cs`. There, the MCP
//! SDK logged every exception thrown by a tool at Error level with a full
//! stacktrace, which floods the console with what is really a normal client
//! facing failure (ERR_FORBIDDEN, ERR_NOT_FOUND, ...). The C# filter downgraded
//! those to a single concise Information line.
//!
//! In Rust there is no ambient exception logger to intercept: the MCP endpoint
//! decides how to log a failed tool call. So the filter becomes an explicit
//! classification helper ([`is_expected`]) plus one logging entry point
//! ([`log_tool_failure`]) that the endpoint calls. Same outcome: 4xx style
//! failures are INFO with no backtrace, genuine 5xx failures are ERROR.

use crate::errors::ToolError;
use std::error::Error;
use tracing_subscriber::EnvFilter;

/// Environment variable read for the log filter, following the tracing convention.
pub const FILTER_ENV: &str = "RUST_LOG";
/// Filter applied when `RUST_LOG` is unset, matching the C# minimum level.
pub const DEFAULT_FILTER: &str = "info";

/// Install the global tracing subscriber. Safe to call more than once: a second
/// call is a no op rather than a panic, so tests and the CLI can both call it.
///
/// Logs go to stderr on purpose: stdout stays clean so `mcp-fs token` can be
/// piped straight into a file.
pub fn init() {
    let filter = EnvFilter::try_from_env(FILTER_ENV)
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

/// Is this failure an expected, client facing one that needs no backtrace?
///
/// True only for a [`ToolError`] whose HTTP mapping is below 500: those are the
/// 4xx style outcomes (forbidden, not found, invalid argument, quota, ...) that
/// the caller caused and can fix. `ERR_INTERNAL_ERROR` and anything that is not
/// a `ToolError` stay unexpected, so they keep ERROR level and full context.
pub fn is_expected(err: &(dyn Error + 'static)) -> bool {
    // Reuse the single definition of "the caller's fault" rather than re-deriving it
    // from a status threshold here: a code mapped to 501 is a server side gap, not an
    // expected client mistake, and only `is_client_error` knows that.
    err.downcast_ref::<ToolError>().is_some_and(ToolError::is_client_error)
}

/// Log a failed tool call at the right level: INFO and concise when expected,
/// ERROR when not.
pub fn log_tool_failure(tool: &str, err: &ToolError) {
    if is_expected(err) {
        tracing::info!(tool = %tool, "tool call failed: {err}");
    } else {
        tracing::error!(tool = %tool, "tool call failed unexpectedly: {err}");
    }
}

/// Log a rejected request (no valid bearer) at INFO: an unauthenticated caller
/// is an expected condition, not a server fault.
pub fn log_unauthenticated(path: &str, err: &ToolError) {
    tracing::info!(path = %path, "rejected request: {err}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_facing_tool_errors_are_expected() {
        for e in [
            ToolError::forbidden("nope"),
            ToolError::not_found("nope"),
            ToolError::invalid_argument("nope"),
            ToolError::unauthenticated("nope"),
            ToolError::project_not_found("p"),
            ToolError::no_clobber("nope"),
            ToolError::path_out_of_bounds("nope"),
        ] {
            assert!(is_expected(&e), "{} must be expected", e.code);
        }
    }

    #[test]
    fn internal_tool_errors_are_not_expected() {
        // ERR_INTERNAL_ERROR maps to 500, so it keeps ERROR level.
        assert!(!is_expected(&ToolError::internal("boom")));
    }

    #[test]
    fn non_tool_errors_are_never_expected() {
        let io = std::io::Error::other("disk on fire");
        assert!(!is_expected(&io));
        let parse: Box<dyn Error> = Box::new("x".parse::<i32>().unwrap_err());
        assert!(!is_expected(parse.as_ref()));
    }

    #[test]
    fn init_is_idempotent() {
        init();
        init();
    }

    #[test]
    fn logging_helpers_do_not_panic() {
        init();
        log_tool_failure("fs.read", &ToolError::not_found("/a.txt"));
        log_tool_failure("fs.read", &ToolError::internal("boom"));
        log_unauthenticated("/mcp", &ToolError::unauthenticated("no bearer token"));
    }
}
