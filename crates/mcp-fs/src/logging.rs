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
///
/// Under `cfg(test)` this always also installs [`capture::CaptureLayer`],
/// regardless of which caller wins the race to be the first to install the
/// process wide global default: `set_global_default` can only ever succeed
/// once per process, and `cargo test` runs every test in this crate in one
/// binary, so whichever test (this crate's own, or a capture-based one in
/// `tools/git.rs`) calls `init` first must not leave the other with no
/// capturing subscriber at all. Both entry points therefore install the
/// exact same composed subscriber.
pub fn init() {
    let filter =
        EnvFilter::try_from_env(FILTER_ENV).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    #[cfg(test)]
    {
        use tracing_subscriber::prelude::*;
        let _ = tracing_subscriber::registry()
            .with(capture::CaptureLayer.with_filter(tracing_subscriber::filter::LevelFilter::TRACE))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_target(false)
                    .with_filter(filter),
            )
            .try_init();
    }
    #[cfg(not(test))]
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .try_init();
    }
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

/// Process wide tracing span and event capture, for tests that need to
/// inspect what a `git.remote` span or a tracing event actually carried
/// (E2E-NEW-151, E2E-NEW-247): `tracing::subscriber::set_global_default` can
/// only succeed once per process, and `spawn_blocking`'s worker threads have
/// no thread-local subscriber of their own, so only a process wide global
/// default (installed once, here, by [`super::init`]) sees everything a
/// remote operation emits regardless of which OS thread it runs on.
#[cfg(test)]
pub(crate) mod capture {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::registry::LookupSpan;

    /// One closed `git.remote` span (or any other span, filtered by the
    /// caller on `name`), with every field it ever recorded, latest value
    /// wins: `on_record` updates the same map `on_new_span` seeded.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct CapturedSpan {
        pub name: String,
        pub fields: BTreeMap<String, String>,
    }

    /// One tracing event, with every field it carried including `message`.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct CapturedEvent {
        pub fields: BTreeMap<String, String>,
    }

    #[derive(Default)]
    struct FieldMap(BTreeMap<String, String>);

    impl Visit for FieldMap {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_string(), format!("{value:?}"));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    pub(crate) struct CaptureLayer;

    static SPANS: OnceLock<Mutex<Vec<CapturedSpan>>> = OnceLock::new();
    static EVENTS: OnceLock<Mutex<Vec<CapturedEvent>>> = OnceLock::new();
    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    // `CAPTURE_LOCK` only ever serializes the handful of tests that call
    // `lock_for_test` against EACH OTHER. The tracing dispatcher, and
    // therefore `CaptureLayer`, is process wide and `cargo test` runs every
    // other test in this crate (including several hundred unrelated clone,
    // push, fetch and pull tests that never call `lock_for_test`) in
    // parallel on other OS threads. Without a gate, any `git.remote` span or
    // event one of those unrelated tests happens to emit while a capturing
    // test's window is open lands in the same global buffer and inflates the
    // count the capturing test observes.
    //
    // The fix is a thread local, not a process wide flag: every test that
    // calls `lock_for_test` (`e2e_new_151`, `e2e_new_247`) runs its whole
    // async body on a dedicated, freshly built single threaded runtime
    // (`with_git_hosts_lock`), so every span and event it causes, including
    // ones from a `spawn_blocking` closure awaited from that same task,
    // opens and closes its lifecycle on that one OS thread. A process wide
    // atomic bool would not help here: it would still be `true` on every
    // OTHER thread for the whole time this test's guard is held, so an
    // unrelated concurrently running test's spans would still pass the
    // gate. A flag scoped to the thread that actually holds the guard is
    // what makes the isolation real: an unrelated test's spans and events
    // always fire on ITS OWN OS thread, where the flag was never set, so
    // they are silently dropped instead of polluting this test's buffer.
    thread_local! {
        static CAPTURING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn is_capturing() -> bool {
        CAPTURING.with(std::cell::Cell::get)
    }

    fn spans_store() -> &'static Mutex<Vec<CapturedSpan>> {
        SPANS.get_or_init(|| Mutex::new(Vec::new()))
    }
    fn events_store() -> &'static Mutex<Vec<CapturedEvent>> {
        EVENTS.get_or_init(|| Mutex::new(Vec::new()))
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            id: &tracing::span::Id,
            ctx: Context<'_, S>,
        ) {
            if !is_capturing() {
                return;
            }
            let mut fields = FieldMap::default();
            attrs.record(&mut fields);
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(fields.0);
            }
        }

        fn on_record(
            &self,
            id: &tracing::span::Id,
            values: &tracing::span::Record<'_>,
            ctx: Context<'_, S>,
        ) {
            if !is_capturing() {
                return;
            }
            if let Some(span) = ctx.span(id) {
                let mut ext = span.extensions_mut();
                if let Some(existing) = ext.get_mut::<BTreeMap<String, String>>() {
                    let mut fields = FieldMap(std::mem::take(existing));
                    values.record(&mut fields);
                    *existing = fields.0;
                }
            }
        }

        fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
            if !is_capturing() {
                return;
            }
            if let Some(span) = ctx.span(&id) {
                let name = span.name().to_string();
                let fields = span
                    .extensions()
                    .get::<BTreeMap<String, String>>()
                    .cloned()
                    .unwrap_or_default();
                spans_store()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(CapturedSpan { name, fields });
            }
        }

        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            if !is_capturing() {
                return;
            }
            let mut fields = FieldMap::default();
            event.record(&mut fields);
            events_store()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CapturedEvent { fields: fields.0 });
        }
    }

    /// Guard returned by [`lock_for_test`]: releases `CAPTURE_LOCK` and
    /// clears the thread local capturing flag on drop, in that order doesn't
    /// matter since both are scoped to this one thread's test.
    pub(crate) struct CaptureGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for CaptureGuard {
        fn drop(&mut self) {
            CAPTURING.with(|c| c.set(false));
        }
    }

    /// Serializes every test that reads or clears the capture buffers: they
    /// are process global (the tracing dispatcher is process wide), so two
    /// such tests running concurrently would observe each other's spans and
    /// events. Also ensures the capturing subscriber is actually installed,
    /// and marks the calling thread as the one whose spans and events
    /// `CaptureLayer` should actually record (see the isolation note above
    /// `CAPTURING`).
    pub(crate) fn lock_for_test() -> CaptureGuard {
        super::init();
        let guard = CAPTURE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        clear();
        CAPTURING.with(|c| c.set(true));
        CaptureGuard { _lock: guard }
    }

    pub(crate) fn clear() {
        spans_store().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        events_store().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    }

    pub(crate) fn spans() -> Vec<CapturedSpan> {
        spans_store().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub(crate) fn events() -> Vec<CapturedEvent> {
        events_store().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
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
