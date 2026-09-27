.PHONY: sync build release test test-e2e-full lint lint-fix format format-check typecheck security check run run-dev clean help

## sync: Fetch dependencies and build the workspace
sync:
	cargo fetch
	cargo build --workspace

## build: Release build of the whole workspace (delegates to build.sh, unmodified)
build:
	./build.sh

## release: Alias for build
release: build

## test: Full test suite (delegates to test.sh, unmodified)
test:
	./test.sh

## test-e2e-full: Full-stack e2e (clone/write/push/pull) against a REAL PostgreSQL + MinIO
# Mandatory infra, never skipped: set MCPFS_TEST_PG_DSN and MCPFS_MINIO_SECRET_KEY first.
# See crates/core/tests/full_stack_e2e.rs's module docs for the full env var list.
test-e2e-full:
	cargo test -p mcp-fs-core --all-features --test full_stack_e2e -- --ignored --nocapture

## lint: Clippy, deny warnings
lint:
	cargo clippy --all-targets --all-features -- -D warnings

## lint-fix: Apply clippy's automatic fixes
lint-fix:
	cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features

## format: Format the workspace with rustfmt
format:
	cargo fmt --all

## format-check: Check formatting without modifying files
format-check:
	cargo fmt --all -- --check

## typecheck: Fast compile check, no codegen
typecheck:
	cargo check --workspace --all-targets

## security: Dependency advisory and license gate
security:
	cargo audit --ignore RUSTSEC-2023-0071
	cargo deny check

## check: Full quality gate (format-check, lint, typecheck, security, test)
check: format-check lint typecheck security test

## run: Boot the server for local development (delegates to run.sh, unmodified)
run:
	./run.sh

## run-dev: Alias for run
run-dev: run

## clean: Remove build artifacts
clean:
	cargo clean

## help: Show this help message
help:
	@grep -E '^## [a-zA-Z_-]+:' $(MAKEFILE_LIST) | sed 's/^## /  /'
