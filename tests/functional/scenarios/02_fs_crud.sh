#!/usr/bin/env bash
# Scénario 02 — fs.write / fs.read / fs.stat / fs.exists
# Tools couverts: admin.create_project, fs.write, fs.read, fs.stat, fs.exists,
#                 fs.create_empty, fs.delete

suite "02 · Filesystem — CRUD basique"

PROJ="ft-crud-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

OUT=$(run_agent "Dans le projet $PROJ, écris le fichier /hello.txt avec le contenu 'Bonjour le monde'.")
assert_contains "write retourne le path" "$OUT" "hello.txt"

OUT=$(run_agent "Lis le fichier /hello.txt du projet $PROJ.")
assert_contains "read retourne le contenu" "$OUT" "Bonjour le monde"

OUT=$(run_agent "Donne-moi le stat de /hello.txt dans $PROJ.")
assert_contains "stat retourne size" "$OUT" "size\|taille\|byte"
assert_contains "stat retourne kind=file" "$OUT" "file"

OUT=$(run_agent "Est-ce que /hello.txt existe dans $PROJ ? Et /ghost.txt ?")
assert_contains "exists vrai pour hello.txt" "$OUT" "oui\|yes\|true\|exist"
assert_contains "exists faux pour ghost.txt" "$OUT" "non\|no\|false\|n'exist\|not"

OUT=$(run_agent "Crée un fichier vide /empty.txt dans $PROJ.")
assert_contains "create_empty confirmé" "$OUT" "empty.txt\|créé\|creat"

OUT=$(run_agent "Supprime /empty.txt dans $PROJ.")
assert_contains "delete confirmé" "$OUT" "supprim\|delet"

run_agent "Supprime le projet $PROJ." >/dev/null
