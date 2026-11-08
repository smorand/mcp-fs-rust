#!/usr/bin/env bash
# Full test suite. S3/MinIO integration tests skip themselves when :9000 is down.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --workspace
# The opt in `rar` feature is off in the default build, so its tests (BL-0130) need their own run.
cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_
