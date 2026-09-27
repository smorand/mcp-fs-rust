.PHONY: sync build release test lint lint-fix format format-check typecheck security check run run-dev clean help

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
