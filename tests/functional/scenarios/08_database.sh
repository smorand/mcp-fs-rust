#!/usr/bin/env bash
# Scénario 08 — db.* et sqlite.* : requêtes SQL sur CSV et SQLite en volume
# Tools couverts: db.query, db.schema, db.sample, db.detect_format,
#                 sqlite.create_table, sqlite.insert, sqlite.query,
#                 sqlite.list_tables, sqlite.schema, sqlite.select

suite "08 · Bases de données — CSV et SQLite"

PROJ="ft-db-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

# ── CSV via DataFusion ──────────────────────────────────────────────────────
run_agent "Dans $PROJ, écris /sales.csv avec :
product,qty,price
Widget,10,9.99
Gadget,5,24.99
Doohickey,3,4.99" >/dev/null

OUT=$(run_agent "Dans $PROJ, utilise db.query pour calculer le total (qty * price) par produit sur /sales.csv, trie par total décroissant.")
assert_contains "query retourne Gadget en tête" "$OUT" "Gadget\|gadget"
assert_contains "query retourne Widget" "$OUT" "Widget\|widget"

OUT=$(run_agent "Dans $PROJ, donne-moi le schéma (colonnes et types) de /sales.csv.")
assert_contains "schema retourne product" "$OUT" "product"
assert_contains "schema retourne qty ou price" "$OUT" "qty\|price"

OUT=$(run_agent "Dans $PROJ, donne-moi un sample de 2 lignes de /sales.csv.")
assert_contains "sample retourne des lignes" "$OUT" "Widget\|Gadget\|Doohickey"

# ── SQLite natif ─────────────────────────────────────────────────────────────
run_agent "Dans $PROJ, crée une table SQLite dans /app.db avec les colonnes : id INTEGER PRIMARY KEY, name TEXT, score REAL." >/dev/null

OUT=$(run_agent "Dans $PROJ, liste les tables de /app.db.")
assert_contains "list_tables voit la table" "$OUT" "table\|tab"

OUT=$(run_agent "Dans $PROJ, insère 3 lignes dans la table de /app.db : Alice/98.5, Bob/87.0, Carol/92.3.")
assert_contains "insert confirmé" "$OUT" "insert\|inséré\|ok\|3"

OUT=$(run_agent "Dans $PROJ, sélectionne tous les enregistrements de la table dans /app.db triés par score décroissant.")
assert_contains "Alice en premier (score max)" "$OUT" "Alice\|alice"

OUT=$(run_agent "Dans $PROJ, quelle est la structure (colonnes) de la table dans /app.db ?")
assert_contains "schema retourne name" "$OUT" "name"
assert_contains "schema retourne score" "$OUT" "score"

run_agent "Supprime le projet $PROJ." >/dev/null
