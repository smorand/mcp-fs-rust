#!/usr/bin/env bash
# Boot the server for local development: config bootstrap, dev keypair, build, serve.
set -euo pipefail
cd "$(dirname "$0")"

# Load local secrets (MinIO keys, GitHub client secret, token key) if present.
# .env is gitignored, so real secrets never reach a commit.
if [ -f .env ]; then
  set -a; . ./.env; set +a
fi

# Bootstrap a personal config on first run (config/local.yaml is gitignored).
if [ ! -f config/local.yaml ]; then
  echo "config/local.yaml not found; copying from config/local.yaml.template"
  cp config/local.yaml.template config/local.yaml
fi

# The keypair must exist before the server can verify a bearer token.
if [ ! -f .keys/jwt.pub ]; then
  echo "Generating a dev keypair in .keys ..."
  cargo run --release --quiet -p mcp-fs -- keys --dir .keys
fi

cargo build --release
exec ./target/release/mcp-fs serve --config config/local.yaml
