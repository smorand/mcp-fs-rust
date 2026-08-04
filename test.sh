#!/usr/bin/env bash
# Full test suite. S3/MinIO integration tests skip themselves when :9000 is down.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --workspace
