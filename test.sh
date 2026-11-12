#!/usr/bin/env bash
# Full test suite. S3/MinIO integration tests skip themselves when :9000 is down.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --workspace
# The opt in `rar` feature is off in the default build, so its tests (BL-0130) need their own run.
cargo test -p mcp-fs-core --lib --features rar -- archive_rar e2e_0080_
# The probe timing tests of SPEC-0091 (e2e_0091_) are #[ignore]d because they flake when run in parallel with
# the rest of the suite; they run alone here.
cargo test -p mcp-fs-core --lib -- --ignored e2e_0091_ --test-threads=1
