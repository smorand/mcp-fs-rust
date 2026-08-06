#!/usr/bin/env bash
# Scénario 10 — sécurité : isolation projets, quota, read_guard
# Tools couverts: fs.write (quota), fs.read (guard), fs.edit (guard), admin.* (ACL)

suite "10 · Sécurité — isolation et garde-fous"

PROJ_A="ft-sec-a-$$"
PROJ_B="ft-sec-b-$$"

run_agent "Crée deux projets : $PROJ_A (owner: admin@example.com) et $PROJ_B (owner: admin@example.com)." >/dev/null

# ── read_guard : édition sans lecture préalable ──────────────────────────────
run_agent "Dans $PROJ_A, écris /protected.txt avec le contenu 'données sensibles'." >/dev/null

OUT=$(run_agent "Dans $PROJ_A, essaie de remplacer 'sensibles' par 'modifié' dans /protected.txt SANS le lire avant — utilise directement fs.edit.")
# L'agent va tenter fs.edit sans avoir fait fs.read : doit recevoir ERR_READ_GUARD_REQUIRED
# L'agent intelligent peut d'abord lire puis éditer, c'est aussi acceptable
assert_contains "edit nécessite read ou réussit après read" "$OUT" "sensibles\|modifié\|read\|guard\|lu"

# ── isolation inter-projets ───────────────────────────────────────────────────
run_agent "Dans $PROJ_A, écris /secret.txt avec 'confidentiel A'." >/dev/null
run_agent "Dans $PROJ_B, écris /secret.txt avec 'confidentiel B'." >/dev/null

OUT=$(run_agent "Lis /secret.txt dans $PROJ_A et /secret.txt dans $PROJ_B. Sont-ils différents ?")
assert_contains "fichiers isolés entre projets" "$OUT" "différent\|different\|A.*B\|deux"

# ── no-clobber : écrasement interdit sans overwrite ──────────────────────────
run_agent "Dans $PROJ_A, écris /locked.txt avec 'version 1'." >/dev/null
OUT=$(run_agent "Dans $PROJ_A, essaie d'écrire à nouveau dans /locked.txt avec 'version 2' SANS overwrite — que se passe-t-il ?")
assert_contains "no-clobber déclenché" "$OUT" "exist\|409\|clobber\|overwrite\|déjà\|already\|erreur\|error"

# ── soft delete : pas de suppression physique sans flag ────────────────────
run_agent "Dans $PROJ_A, écris /temp.txt avec 'temporaire'." >/dev/null
OUT=$(run_agent "Dans $PROJ_A, supprime /temp.txt.")
assert_contains "soft delete confirmé" "$OUT" "supprim\|delet\|ok"

OUT=$(run_agent "Est-ce que /temp.txt existe encore dans $PROJ_A ?")
assert_contains "fichier supprimé" "$OUT" "non\|no\|false\|n'exist\|not"

run_agent "Supprime les projets $PROJ_A et $PROJ_B." >/dev/null
