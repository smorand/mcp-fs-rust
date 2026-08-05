# Backlog

## Agent CLI

## Outils de génération de documents

### Générateur DOCX depuis Markdown ou HTML

*Implémenté: `doc.to_docx`.*

### Générateur PPTX depuis Markdown ou HTML

*Implémenté: `doc.to_pptx`.*

---

## Éditeur HTML single-page (docx-style et pptx-style)

Permet au LLM de créer et modifier des documents HTML single-page dans un volume,
le tout avec un mini-navigateur éditable qui synchronise les modifications manuelles
en temps réel dans le filesystem.

Deux modes de document:
- **docx-style**: document CMS-like avec en-têtes, images, sections. Rendu Word-like dans le navigateur.
- **pptx-style**: présentation avec slides navigables (précédent/suivant), chaque `<section>` = une slide.

Fonctionnalités:
- Template initial (HTML ou YAML) pour amorcer la structure.
- Le LLM travaille sur l'HTML brut via `fs.*` tools.
- Un mini-serveur local sert l'HTML et watch les changements fichier (WebSocket).
  Les éditions manuelles dans le navigateur écrivent dans le fichier du volume.
- Single-page (CSS et JS inline): pas de dépendances externes, facile à télécharger.
- Tools MCP envisagés: `doc.open_editor` (démarre le mini-serveur + ouvre le navigateur),
  `doc.create_html_doc`, `doc.create_html_slides`.

*Sujet complexe, à cadrer en session dédiée.*
