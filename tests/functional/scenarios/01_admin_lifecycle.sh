#!/usr/bin/env bash
# Scénario 01 — admin.* : cycle de vie projet et membres
# Tools couverts: admin.create_project, admin.list_projects, admin.add_member,
#                 admin.list_members, admin.remove_member, admin.delete_project

suite "01 · Admin — cycle de vie projet"

PROJ="ft-admin-$$"

OUT=$(run_agent "Crée un projet nommé $PROJ dont je suis le propriétaire (utilise admin@example.com comme owner).")
assert_contains "create_project retourne l'id" "$OUT" "$PROJ"

OUT=$(run_agent "Liste tous mes projets.")
assert_contains "list_projects voit $PROJ" "$OUT" "$PROJ"

OUT=$(run_agent "Ajoute alice@test.com comme membre du projet $PROJ.")
assert_contains "add_member confirmé" "$OUT" "alice"

OUT=$(run_agent "Liste les membres du projet $PROJ.")
assert_contains "list_members voit alice" "$OUT" "alice"

OUT=$(run_agent "Retire alice@test.com du projet $PROJ.")
assert_contains "remove_member confirmé" "$OUT" "retir\|remov\|supprim"

OUT=$(run_agent "Supprime le projet $PROJ.")
assert_contains "delete_project confirmé" "$OUT" "supprim\|delet\|ok"

OUT=$(run_agent "Liste tous mes projets.")
assert_not_contains "projet absent après suppression" "$OUT" "$PROJ"
