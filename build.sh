#!/usr/bin/env bash
# Release build of the whole workspace.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release
