# Backlog

## Agent CLI

## Outils de génération de documents

### Générateur DOCX depuis Markdown ou HTML

*Implémenté: `doc.to_docx`.*

### Générateur PPTX depuis Markdown ou HTML

*Implémenté: `doc.to_pptx`.*

---

## ~~Éditeur HTML single-page~~ ✅ Implémenté

Trois tools MCP dans la famille `doc.*`:
- `doc.open_editor` — démarre un mini-serveur HTTP+WebSocket par éditeur, crée le fichier si absent (mode `doc` ou `slides`), ouvre le navigateur, retourne `editor_id` + URL.
- `doc.close_editor` — arrête l'éditeur et libère le port.
- `doc.list_editors` — liste les éditeurs actifs.

Sync bidirectionnel: browser → volume via WS `save`, volume → browser via poll mtime 500ms + WS `reload`.
Activé avec le flag `--doc` (même feature que `doc.to_docx` / `doc.to_pptx`).
17 tests dans `tools::editor`.
