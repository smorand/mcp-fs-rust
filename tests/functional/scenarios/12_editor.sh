#!/usr/bin/env bash
# Scénario 12 — doc.open_editor / doc.close_editor / doc.list_editors
# Tools couverts: doc.open_editor, doc.close_editor, doc.list_editors
#
# Prérequis : serveur démarré avec --doc (ou doc.enabled: true dans la config)

suite "12 · Éditeur HTML — open/list/close"

PROJ="ft-editor-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

# Vérifier si doc.open_editor est disponible
PROBE=$(run_agent "Liste tous les outils doc.* disponibles.")
if ! echo "$PROBE" | grep -qi "open_editor\|list_editors"; then
  skip "doc.open_editor" "--doc non activé"
  skip "doc.close_editor" "--doc non activé"
  skip "doc.list_editors" "--doc non activé"
  run_agent "Supprime le projet $PROJ." >/dev/null
  return
fi

# ── list vide avant tout ouverture ───────────────────────────────────────────
OUT=$(run_agent "Liste tous les éditeurs HTML ouverts (doc.list_editors).")
assert_contains "liste vide au départ" "$OUT" "vide\|empty\|\[\]\|0\|aucun\|no editor"

# ── open en mode doc (fichier créé) ──────────────────────────────────────────
OUT=$(run_agent "Ouvre un éditeur HTML pour /article.html dans $PROJ en mode doc. Donne-moi l'editor_id et l'URL.")
assert_contains "open_editor retourne une URL" "$OUT" "127.0.0.1\|localhost\|http"
assert_contains "open_editor retourne created=true" "$OUT" "créé\|creat\|true\|nouveau"

OUT=$(run_agent "Est-ce que /article.html existe dans $PROJ ?")
assert_contains "fichier créé automatiquement" "$OUT" "oui\|yes\|true\|exist"

# ── idempotence ───────────────────────────────────────────────────────────────
OUT=$(run_agent "Ouvre à nouveau un éditeur pour /article.html dans $PROJ mode doc — quel est l'editor_id ?")
# Le second appel doit retourner le même editor_id
FIRST_URL=$(run_agent "Liste les éditeurs ouverts." | grep -o "127.0.0.1:[0-9]*" | head -1)
assert_contains "second open retourne created=false" "$OUT" "false\|exist\|déjà\|already"

# ── open en mode slides ───────────────────────────────────────────────────────
OUT=$(run_agent "Ouvre un éditeur pour /deck.html dans $PROJ en mode slides.")
assert_contains "open slides retourne URL" "$OUT" "127.0.0.1\|localhost\|http"

OUT=$(run_agent "Est-ce que /deck.html existe dans $PROJ ?")
assert_contains "deck créé" "$OUT" "oui\|yes\|true\|exist"

OUT=$(run_agent "Lis le contenu de /deck.html dans $PROJ.")
assert_contains "deck contient section" "$OUT" "section\|slide"

# ── list montre les deux éditeurs ─────────────────────────────────────────────
OUT=$(run_agent "Liste tous les éditeurs HTML ouverts.")
assert_contains "list voit article.html" "$OUT" "article"
assert_contains "list voit deck.html" "$OUT" "deck"

# ── close ─────────────────────────────────────────────────────────────────────
OUT=$(run_agent "Ferme l'éditeur pour /article.html dans $PROJ (utilise son editor_id pour doc.close_editor).")
assert_contains "close confirmé" "$OUT" "fermé\|clos\|closed\|true\|ok"

OUT=$(run_agent "Liste les éditeurs ouverts.")
assert_not_contains "article absent après close" "$OUT" "article.html"
assert_contains "deck toujours ouvert" "$OUT" "deck"

# ── close l'éditeur restant ───────────────────────────────────────────────────
OUT=$(run_agent "Ferme tous les éditeurs ouverts.")
assert_contains "tous fermés" "$OUT" "fermé\|closed\|ok"

# ── close avec id inconnu ─────────────────────────────────────────────────────
OUT=$(run_agent "Essaie de fermer un éditeur avec l'id '00000000-0000-0000-0000-000000000000' — que se passe-t-il ?")
assert_contains "close inconnu retourne erreur" "$OUT" "erreur\|error\|not found\|introuvable"

run_agent "Supprime le projet $PROJ." >/dev/null
