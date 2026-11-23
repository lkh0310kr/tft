---
tags:
  - tft/meta
---
# Web

The web app is a second shell over the same Rust core; it is not a reduced
product. Inside the browser viewport it must match the desktop's tokens, metrics
and behavior. Browser chrome outside the viewport is the only accepted
difference.

## Storage

- Google Drive only, `drive.file` scope.
- Picker selects the vault folder; `drive.file` grants the selected folder or
  app-created files.
- Source and core treat paths as vault-relative strings; the Drive adapter owns
  file IDs, ETags and HTTP.
- `workspace.json` and other UI-only state stay in the browser (localStorage or
  IndexedDB), not in the Drive vault.

## Parity matrix

| Area | Desktop | Web now | Next |
|---|---|---|---|
| Tokens/metrics | `core::design` | all 131 token values as CSS vars | shared metrics export |
| Shell | left dock, tabs, view header | Files/Search docks, tab strip, view header | exact chrome |
| Files | `FileTree`, 25 px rows | same `FileTree`, sort/collapse | drag |
| Reading | reading model | `markdown::render_html` + CSS | shared reading model |
| Editing | Live Preview + Source | Source textarea, save/blur | Live Preview |
| Search | full search pane | filename + indexed full text | snippets/metadata filters |
| Settings | Appearance + Editor | same single modal | exact window layout |
| Quick switcher | GPUI dialog | Ctrl+O modal | same rows/keys |
| File ops | context menu | new/rename/delete/context menu | move/copy |
| Graph/canvas/PDF | GPUI views | not yet | DOM renderers |
| Window chrome | native frame | browser chrome | PWA/standalone |

## Phases

1. Drive index, tree, Markdown read, Source edit, tabs, token shell. **Current.**
2. Live Preview, reading model, menus, search pane, settings, quick switcher.
3. Canvas, graph, PDF, exports, offline cache.
4. Vercel production, custom domain, OAuth origins, performance pass.

The shared core stays framework-neutral. `apps/web` owns Leptos and the Drive
adapter; no desktop feature is reimplemented in a second parser or editor model.
The rebuild plan and its acceptance gates are in [[Web-Rebuild]].

## Security

- `drive.file` only; the Picker grants the PKMS folder.
- OAuth token stays in memory; the client secret is never used.
- The production bundle carries no API key.
- Rendered note HTML is sanitized in the browser: scripts, frames, forms and
  `on*` attributes are removed.
- Vercel serves static files only; no vault data leaves Drive for the host.
- CSP, HSTS, referrer and permissions policies are in `apps/web/vercel.json`.
