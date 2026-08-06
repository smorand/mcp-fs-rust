#!/usr/bin/env bash
# Scénario 13 — flux de travail multi-étapes (chaînage d'outils)
# Simule un vrai usage LLM : l'agent enchaîne plusieurs tools dans une seule session.
# Tools couverts: admin.*, fs.write, fs.read, fs.edit, fs.glob, fs.grep,
#                 fs.extract_text, db.query — tout en un flux conversationnel.

suite "13 · Flux de travail — session multi-outils"

PROJ="ft-workflow-$$"

# Un seul bloc de prompts enchaînés dans la même session stdin
OUT=$(run_agent "Crée un projet $PROJ avec toi comme owner (admin@example.com).
Écris trois fichiers dans $PROJ :
- /products.csv : name,category,price\nLaptop,Electronics,999\nPhone,Electronics,599\nDesk,Furniture,299\nChair,Furniture,199
- /summary.md : # Inventaire\nCe fichier résume notre stock.
- /config.json : {\"version\":1,\"active\":true}
Maintenant, trouve tous les fichiers .csv dans $PROJ avec glob.
Requête /products.csv avec db.query pour calculer le prix moyen par catégorie.
Mets à jour /summary.md : remplace 'stock' par 'inventaire complet' et ajoute une ligne 'Mis à jour par le LLM.'
Montre-moi le contenu final de /summary.md.
")

assert_contains "projet créé" "$OUT" "$PROJ"
assert_contains "fichiers écrits" "$OUT" "products.csv\|csv"
assert_contains "glob trouve csv" "$OUT" "products.csv"
assert_contains "query retourne Electronics ou Furniture" "$OUT" "Electronics\|Furniture\|électronique\|meuble"
assert_contains "prix moyen calculé" "$OUT" "[0-9]\{2,\}\|average\|moyen"
assert_contains "summary mis à jour" "$OUT" "inventaire complet\|Mis à jour"

run_agent "Supprime le projet $PROJ." >/dev/null
