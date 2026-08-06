#!/usr/bin/env bash
# Scénario 06 — fs.read_lines / fs.read_many / fs.read_bytes / fs.count_lines
#               fs.read_section / fs.hash
# Tools couverts: fs.read_lines, fs.read_many, fs.read_bytes, fs.count_lines,
#                 fs.read_section, fs.hash

suite "06 · Lecture avancée — slices, bytes, hash"

PROJ="ft-read-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

run_agent "Dans $PROJ, écris /numbers.txt avec exactement ce contenu (10 lignes) :
ligne 1
ligne 2
ligne 3
ligne 4
ligne 5
ligne 6
ligne 7
ligne 8
ligne 9
ligne 10" >/dev/null

OUT=$(run_agent "Dans $PROJ, lis les lignes 3 à 5 de /numbers.txt.")
assert_contains "read_lines retourne ligne 3" "$OUT" "ligne 3"
assert_contains "read_lines retourne ligne 5" "$OUT" "ligne 5"
assert_not_contains "read_lines exclut ligne 1" "$OUT" "ligne 1"

OUT=$(run_agent "Dans $PROJ, compte le nombre de lignes de /numbers.txt.")
assert_contains "count_lines = 10" "$OUT" "10"

OUT=$(run_agent "Dans $PROJ, lis simultanément /numbers.txt et /ghost.txt — que retourne read_many ?")
assert_contains "read_many isole l'erreur" "$OUT" "ghost\|error\|erreur\|not found"
assert_contains "read_many retourne numbers.txt" "$OUT" "ligne 1"

OUT=$(run_agent "Dans $PROJ, calcule le hash SHA-256 de /numbers.txt.")
assert_contains "hash retourne une valeur hex" "$OUT" "[0-9a-f]\{10,\}"

OUT=$(run_agent "Dans $PROJ, lis les octets bruts de /numbers.txt (base64).")
assert_contains "read_bytes retourne base64" "$OUT" "base64\|bytes\|byt"

run_agent "Supprime le projet $PROJ." >/dev/null
