# Specification Backlog

Ideas raised during specification work and deliberately deferred. Each entry records
why it was deferred and which spec surfaced it. Nothing here is a commitment.

## BL-001: OpenTelemetry instrumentation

**Description.** Replace or supplement the current stderr-only `tracing` setup with an
OpenTelemetry pipeline: OTLP span export behind a config switch, plus rolling file logs.
Spans on the boundaries that matter here: MCP tool calls (tool name, outcome, duration),
relational queries (statement preview, rows, duration), blob operations, and identity
verification. Never on token contents, key material or DSN contents.

**Current state.** Not implemented. The only telemetry dependency in the workspace is
`tracing-subscriber` (`Cargo.toml:98`, `crates/mcp-fs/Cargo.toml:79`). `logging.rs:29-37`
installs a plain `fmt` subscriber writing to stderr, filtered by `RUST_LOG` with a default
of `info`. There is no OTLP exporter, no span export and no file appender.

**Rationale for deferral.** The platform foundation specification is a retro-specification
of shipped behaviour. Writing OTel requirements into it would convert a document that
describes the system into a document that changes it, and would put a large new dependency
surface inside a spec whose value is that it asserts nothing untrue about the code.

**Suggested by.** `2026-09-18_17-37-46-platform-foundation.md`, §7.5, DEC-005.

## BL-002: Specification of the `crates/agent` CLI client

**Description.** A retro-specification of the interactive CLI agent: its config schema, the
LLM streaming and tool-calling loop, the wrap-aware line editor, markdown-to-ANSI rendering,
and the terminal invariants it depends on.

**Current state.** Implemented and shipping (`crates/agent/src/`: `main.rs`, `mcp.rs`,
`llm.rs`, `input.rs`, `ui.rs`, `session.rs`, `spinner.rs`, `config.rs`), documented in
`.agent_docs/agent.md`.

**Rationale for deferral.** The agent is an MCP **client**, not part of the server product
(`crates/agent/src/main.rs:3-5`). Its real value is as the end-to-end exerciser of the tool
surface, which the server specs already assert directly. Specifying it competes for effort
with the seven remaining server specs.

**Suggested by.** The eight-spec split, DEC-001.

## BL-003: Shared session state across replicas

**Description.** Make the write quota and the read-before-write guard shared rather than
per process, so that horizontal scaling does not silently change their semantics.

**Current state.** Session state is an in-memory map keyed by `(person, project_id)`
(`crates/mcp-fs/src/safety.rs:2-4,79-86`). With more than one replica, a caller's quota is
effectively multiplied by the replica count and the read guard can be satisfied on one
replica and enforced on another.

**Rationale for deferral.** This is a product decision about the intended deployment
topology, not a defect in the current single-process design. Recorded as TBD-001 in the
platform foundation spec rather than solved there.

**Suggested by.** `2026-09-18_17-37-46-platform-foundation.md`, §15 TBD-001.
