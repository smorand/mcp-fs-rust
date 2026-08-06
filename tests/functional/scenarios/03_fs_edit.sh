#!/usr/bin/env bash
# Scénario 03 — fs.edit / fs.multi_edit / fs.apply_patch / fs.overwrite
# Tools couverts: fs.write, fs.read, fs.edit, fs.multi_edit, fs.overwrite,
#                 fs.insert_at_line, fs.append

suite "03 · Filesystem — édition et patch"

PROJ="ft-edit-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

run_agent "Dans $PROJ, écris /poem.txt avec ce contenu exact :
Roses are red
Violets are blue
Sugar is sweet
And so are you" >/dev/null

OUT=$(run_agent "Dans le projet $PROJ, remplace 'red' par 'scarlet' dans /poem.txt et montre-moi le diff.")
assert_contains "edit retourne un diff" "$OUT" "scarlet\|diff\|\-red\|\+scarlet"

OUT=$(run_agent "Lis /poem.txt dans $PROJ pour vérifier la modification.")
assert_contains "modification persistée" "$OUT" "scarlet"

OUT=$(run_agent "Dans $PROJ, ajoute la ligne 'And poems are fun' à la fin de /poem.txt.")
assert_contains "append confirmé" "$OUT" "append\|ajout\|ok\|bytes"

OUT=$(run_agent "Lis /poem.txt dans $PROJ.")
assert_contains "ligne ajoutée présente" "$OUT" "poems are fun"

OUT=$(run_agent "Dans $PROJ, insère la ligne 'HEADER LINE' en première position dans /poem.txt.")
assert_contains "insert_at_line confirmé" "$OUT" "insert\|inséré\|ok"

OUT=$(run_agent "Lis /poem.txt dans $PROJ — montre les 3 premières lignes.")
assert_contains "header en première ligne" "$OUT" "HEADER LINE"

run_agent "Supprime le projet $PROJ." >/dev/null
