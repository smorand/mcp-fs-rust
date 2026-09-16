#!/usr/bin/env bash
# Scénario 26 : document as a service, mode api
# Tools couverts: fs.write_bytes (via /upload), fs.list_dir, fs.read
#
# Même chaîne que le scénario 25, mais à travers HTTP : scripts/doc_service_fake.py
# sert de convertisseur et vérifie l'en-tête d'authentification. Les deux modes
# passent ainsi par le même code serveur.
#
# Prérequis : python3 et doc-convert (le faux service délègue au vrai binaire).

suite "26 · Document service, mode api (faux service HTTP)"

# shellcheck source=/dev/null
source "$(dirname "${BASH_SOURCE[0]}")/_doc_service_common.sh"

if ! command -v python3 >/dev/null 2>&1; then
  skip "doc_service api" "python3 absent du PATH"
  return
fi
if ! command -v doc-convert >/dev/null 2>&1; then
  skip "doc_service api" "doc-convert absent du PATH (le faux service l'appelle)"
  return
fi

FAKE_PORT=8099
FAKE_TOKEN="s3cret-fake-token"
FAKE_PID=""

python3 scripts/doc_service_fake.py \
  --port "$FAKE_PORT" \
  --auth-header X-Convert-Key \
  --auth-token "$FAKE_TOKEN" >/tmp/mcp-fs-doc-fake.log 2>&1 &
FAKE_PID=$!
for _ in $(seq 1 50); do
  curl -fsS -m 2 "http://127.0.0.1:$FAKE_PORT/" >/dev/null 2>&1 && break
  sleep 0.2
done

if ! curl -fsS -m 2 "http://127.0.0.1:$FAKE_PORT/" >/dev/null 2>&1; then
  skip "doc_service api" "faux service injoignable sur le port $FAKE_PORT"
  kill -TERM "$FAKE_PID" 2>/dev/null || true
  return
fi

DOC_BLOCK="doc_service:
  enabled: true
  mode: api
  api:
    url: http://127.0.0.1:$FAKE_PORT/convert
    auth_header: X-Convert-Key
    auth_token: \"$FAKE_TOKEN\"
    response_field: \"\"
    timeout_secs: 900"

if ! ds_start 5103 "$DOC_BLOCK"; then
  skip "doc_service api" "serveur dédié impossible à démarrer"
  ds_stop
  kill -TERM "$FAKE_PID" 2>/dev/null || true
  return
fi

FIXTURE="$DS_DIR/sample.pdf"
ds_sample_pdf "$FIXTURE"

# ── upload avec le flag, conversion par le faux service ──────────────────────
OUT=$(ds_upload "$FIXTURE" "true")
assert_contains "upload accepté" "$OUT" "/sample.pdf"
assert_contains "compagnon annoncé dans la réponse" "$OUT" "/sample.md"

OUT=$(ds_mcp fs.list_dir "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/\"}")
assert_contains "source et compagnon listés" "$OUT" "sample.md"

OUT=$(ds_mcp fs.read "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/sample.md\",\"line_numbered\":false}")
assert_contains "markdown non vide" "$OUT" "hello mcpfs"

# ── le token est bien exigé : un mauvais token doit faire échouer la conversion,
#    sans perdre le fichier déjà stocké ─────────────────────────────────────────
ds_stop
DOC_BLOCK="${DOC_BLOCK/$FAKE_TOKEN\"/mauvais-token\"}"
if ds_start 5103 "$DOC_BLOCK"; then
  ds_sample_pdf "$DS_DIR/sample.pdf"
  OUT=$(ds_upload "$DS_DIR/sample.pdf" "true")
  assert_contains "mauvais token signalé" "$OUT" "401"
  OUT=$(ds_mcp fs.exists "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/sample.pdf\"}")
  assert_contains "upload conservé malgré l'échec" "$OUT" "true"
else
  skip "doc_service api auth" "serveur dédié impossible à redémarrer"
fi

ds_stop
kill -TERM "$FAKE_PID" 2>/dev/null || true
