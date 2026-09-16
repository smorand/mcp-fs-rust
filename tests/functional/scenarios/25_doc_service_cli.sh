#!/usr/bin/env bash
# Scénario 25 : document as a service, mode cli
# Tools couverts: fs.write_bytes (via /upload), fs.list_dir, fs.read, fs.documentize
#
# Prérequis : doc-convert sur le PATH. Le scénario démarre son propre serveur
# (port 5102) avec doc_service.enabled + mode cli, et le coupe à la fin.

suite "25 · Document service, mode cli (doc-convert)"

# shellcheck source=/dev/null
source "$(dirname "${BASH_SOURCE[0]}")/_doc_service_common.sh"

if ! command -v doc-convert >/dev/null 2>&1; then
  skip "doc_service cli" "doc-convert absent du PATH"
  return
fi

DOC_BLOCK='doc_service:
  enabled: true
  mode: cli
  cli:
    command: ["doc-convert", "--stdout", "--quiet", "{document}"]
    timeout_secs: 900'

if ! ds_start 5102 "$DOC_BLOCK"; then
  skip "doc_service cli" "serveur dédié impossible à démarrer"
  ds_stop
  return
fi

FIXTURE="$DS_DIR/sample.pdf"
ds_sample_pdf "$FIXTURE"

# ── upload avec le flag : la source et son compagnon ─────────────────────────
OUT=$(ds_upload "$FIXTURE" "true")
assert_contains "upload accepté" "$OUT" "/sample.pdf"
assert_contains "compagnon annoncé dans la réponse" "$OUT" "/sample.md"

# ── les deux fichiers sont bien dans le volume ───────────────────────────────
OUT=$(ds_mcp fs.list_dir "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/\"}")
assert_contains "source listée" "$OUT" "sample.pdf"
assert_contains "compagnon listé" "$OUT" "sample.md"

# ── le markdown produit par le convertisseur est non vide ────────────────────
OUT=$(ds_mcp fs.read "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/sample.md\",\"line_numbered\":false}")
assert_contains "markdown non vide" "$OUT" "hello mcpfs"

# ── fs.documentize : la surface de reprise, no-clobber par défaut ────────────
OUT=$(ds_mcp fs.documentize "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/sample.pdf\"}")
assert_contains "documentize refuse d'écraser" "$OUT" "ERR_NO_CLOBBER"

OUT=$(ds_mcp fs.documentize "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/sample.pdf\",\"overwrite\":true}")
assert_contains "documentize avec overwrite" "$OUT" "/sample.md"

# ── extension inéligible : refus avant toute écriture ────────────────────────
echo "juste du texte" > "$DS_DIR/notes.txt"
OUT=$(ds_upload "$DS_DIR/notes.txt" "true")
assert_contains "extension inéligible rejetée" "$OUT" "ERR_NOT_SUPPORTED"

OUT=$(ds_mcp fs.exists "{\"mount_id\":\"$DS_PROJ\",\"path\":\"/notes.txt\"}")
assert_contains "rien écrit pour l'inéligible" "$OUT" "false"

ds_stop
