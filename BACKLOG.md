# Backlog

## Agent CLI

## Outils de génération de documents

### Générateur DOCX depuis Markdown ou HTML

Convertit un fichier Markdown ou HTML présent dans un volume en fichier `.docx`, avec support
d'un template Word optionnel pour la mise en forme (styles, en-tête, pied de page, logo).

- Tool MCP: `doc.to_docx`
- Params: `mount_id`, `src_path` (Markdown ou HTML), `dst_path` (.docx), `template_path?` (template .docx dans le volume)
- Implémentation: `pandoc` (disponible sur le système) appelé en subprocess avec `--reference-doc` pour le template
- Si pas de template: génère un .docx avec les styles par défaut de pandoc
- Famille `doc.*` à créer, activée par `--doc` / `doc.enabled: true` dans le YAML

### Générateur PPTX depuis Markdown ou HTML

Génère une présentation PowerPoint depuis un fichier Markdown ou HTML présent dans un volume.

- Tool MCP: `doc.to_pptx`
- Params: `mount_id`, `src_path` (Markdown ou HTML), `dst_path` (.pptx), `template_path?` (template .pptx dans le volume)
- Format Markdown: `---` = séparateur de slide, `# Titre` = titre de slide, bullets = contenu
- Format HTML: structure en sections `<section>` ou `<h1>`/`<h2>` selon les conventions pandoc
- Implémentation: `pandoc` avec writer `pptx` + `--reference-doc` pour le template
- Même famille `doc.*` que le générateur DOCX
