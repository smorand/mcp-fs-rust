---
Id: BL-0066
Title: OpenTelemetry instrumentation
Nature: FEAT
Source: split out of the legacy BL-0013 file (its item BL-001) on 2026-10-19; suggested by SPEC-0002 §7.5, DEC-005
Created: 2026-10-19
Severity: Low
---

## What
Replace or supplement the stderr-only `tracing` setup with an OpenTelemetry pipeline: OTLP span
export behind a config switch, plus rolling file logs. Spans on the boundaries that matter: MCP
tool calls (tool name, outcome, duration), relational queries (statement preview, rows,
duration), blob operations, identity verification. Never on token contents, key material or DSN
contents.

## Why
No traces or exported telemetry exist; diagnosing a slow or failing call means reading stderr.

## Evidence
Audit 2026-10-19: `Cargo.toml:125-126` carries only `tracing` and `tracing-subscriber`;
`logging.rs` installs a plain `fmt` subscriber to stderr with no OTLP exporter and no file appender.

## Notes
Deferred originally because SPEC-0002 is a retro-specification of shipped behaviour. Needs its own
FEAT spec. The legacy file's second item (spec of `crates/agent`) is BL-0014.
