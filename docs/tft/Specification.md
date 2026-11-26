---
aliases:
  - Spec
tags:
  - tft/meta
---
# TFT — project specification

TFT is a native Rust/GPUI knowledge base that opens an existing Obsidian vault **in place**.
There is no import step, no conversion, and no second copy of any file. This note is the
normative description of what the project implements. The probe transcripts and every measured
value behind it are kept in `tools/obsidian-probe/reference/observed-behavior.md`; when this note
and a probe disagree, the probe wins and this note is corrected.

## 1. Invariants

1. **The vault folder is the source of truth.** TFT never writes a file it did not change.
   Line endings, a byte-order mark, JSON key order, unknown YAML/JSON keys, and trailing-newline
   state survive a read/write round trip.
2. **One Markdown parser.** `markdown` (`tft_markdown`) feeds the editor, reading view, metadata cache, and
   link rewriting. No second tokenizer is allowed for any of them.
3. **Obsidian identifiers are exact.** Command ids, `.obsidian` file names and config keys,
   `workspace.json` node shapes, canvas JSON, hotkey spellings and default hotkeys are the strings
   Obsidian uses; `crates/commands/data/commands.txt` is the inventory.
4. **Clean room.** Behavior is implemented from the probe record and public documentation. No
   Obsidian code, CSS, or icons are copied.
5. **No native platform APIs in shared crates.** GPUI app entry points live in `apps/desktop` and `apps/web`; `crates/app` and `crates/domain` remain window-free and testable without a platform runner.

## 2. Repository layout

```
apps/desktop        GPUI Desktop application (package desktop; binary tft)
apps/web            GPUI WebAssembly application (Google Drive host)
apps/web-leptos     preserved Leptos implementation
assets              icons and fonts shared by desktop and the future web app
crates/app          shared application state/use cases and wasm-safe services
crates/domain       canonical VaultData graph and domain identities/invariants
crates/vault        vault scanning, file kinds, .obsidian reads/writes
crates/markdown     CommonMark + GFM + Obsidian Markdown, one spanned tree
crates/cache        CachedMetadata and the link index/resolver
crates/formats      on-disk formats: configs, canvas, properties, migration
crates/commands     command ids, registry, default hotkeys
crates/text         editor text model: buffer, selections, undo, edit plans
crates/editor…      (inside desktop) the editor view, Live Preview, tables
crates/suggest      completion triggers, candidates, accepted-link text
crates/search       query grammar, matcher, fuzzy search, exclusions
crates/canvas       canvas viewport/geometry/hit-test/edit model
crates/pdf          PDF pages, outline, text, #page= subpath (hayro)
crates/math         TeX typeset on RaTeX (KaTeX-compatible)
crates/mermaid      Mermaid layout through limu
crates/highlight    Prism tokenizer and 85 language grammars
crates/export       Export to PDF (krilla) and Export as image (tiny-skia)
crates/audio        audio playback and video headers
crates/paste        attachment naming, text/uri-list decoding
crates/spell        Hunspell spellcheck, dictionaries, custom words
crates/control      control-socket wire protocol and blocking client
crates/cli          flags, file/URI arguments, one-window handover
crates/bench        synthetic vault generator and index timer
```

The Rust library names remain `tft_*` even though package and directory names do not carry the
prefix; this keeps `use tft_text::…` imports stable and avoids colliding with the app's own
modules (`control`, `search`, `canvas`, `pdf`, `math`, `mermaid`, `suggest`).

`core`, `markdown`, `formats`, `cache`, `text`, `search`, `suggest`, `canvas`, `commands`,
`highlight`, `paste`, `pdf`, `math`, `mermaid` and `export` build for
`wasm32-unknown-unknown`; `cargo wasm-check` checks that lane. `vault`, `audio`, `spell`,
`control`, `cli` and `apps/desktop` stay native. The future web app consumes the same pure
crates; Google Drive is its storage, not the desktop's.

The web shell is not a reduced reimplementation: inside the browser viewport it uses the same
`core::design` tokens and metrics, the same file tree, tab group, reading/source views and
editor behavior as the desktop. Browser chrome outside the viewport is the one unavoidable
difference. Shared UI data lives in `core`; DOM widgets are a second implementation of the
same specs, never a second product.

## 3. Vault and configuration

- A vault is a folder. `.obsidian/` is read when present; its absence means defaults.
- TFT preserves every unknown key in every modelled config file. A config write changes only the
  keys the action changed.
- Modelled files include `app.json`, `appearance.json`, `workspace.json`, `hotkeys.json`,
  `graph.json`, `types.json`, `switcher.json`, `page-preview.json`, `canvas.json`,
  `core-plugins.json`, and the per-vault spellcheck list.
- **Core plugins are always on.** `core_plugins.json` is no longer read by the app; the
  Core plugins settings page is gone. Plugin behavior code stays modular.
- **Settings are fixed to the Obsidian defaults below:**
  - new notes: vault root;
  - attachments: vault root;
  - link format: shortest path;
  - internal links are prompted before rewriting on rename/move;
  - `[[Wikilinks]]` are on;
  - excluded-file rules are empty and are not applied;
  - deleted files go to the system trash;
  - confirm-before-delete is on.
- `workspace.json` records the tab/split tree in Obsidian's shape. A `"stacked": true` group is
  read as horizontal tabs and the key is dropped on the next write.
- Theme output is a fixed pair of dark/light palettes (131 tokens). There is no `theme.css`,
  community-theme, snippet, accent, font, or app-icon customization. Light/Dark/System is the
  whole Appearance page.

## 4. Markdown and reading

`tft-markdown` parses CommonMark (all 652 spec examples), the GitHub extensions Obsidian uses, and
Obsidian syntax into one tree with byte spans. The same tree builds the reading view, cache,
editor decorations, and link rewriting.

- Callouts, wikilinks (`[[Note]]`, `[[Note#Heading]]`, `[[Note^block]]`, aliases and subpaths),
  embeds (`![[…]]`), tags, math (`$x$`, `$$x$$`), fenced code, task lists, tables, frontmatter,
  footnotes/definitions, comments, raw HTML and Obsidian's Markdown extensions are supported.
- Reading view renders that tree to HTML; the corpus under `crates/markdown/tests/corpus` holds
  Obsidian's own renderings and is a ratchet.
- Image embeds honor width/height, nested lists/quotes, remote `http(s)` images, and PDF subpaths.
- Code fences are highlighted by Prism's grammar for the fence language; an unknown language is
  shown as plain code.
- Spellcheck covers the prose the parser leaves to it, using SCOWL's en_US by default and system
  Hunspell dictionaries for other languages; users can keep a custom word list.

## 5. Editing

The editor is one view over `tft-text`; the buffer, selections, undo and edit plans live there.

- **Reading / Live Preview / Source** modes. New tabs default to Editing view and Live Preview.
  Source mode shows raw Markdown with syntax highlighting.
- Find and replace, multiple cursors, auto-pairing of brackets and Markdown syntax, smart list
  continuation, tab indentation at width 4, readable line length, folding of headings and lists,
  line numbers off, indentation guides on, right-to-left off.
- Tables: Obsidian's cell editing, row/column commands, alignment and drag handles.
- Properties are an editable typed table at the top of a note; edits rewrite only the lines they
  change. YAML types are stored in `types.json`.
- Links are rewritten on rename/move/copy according to the fixed shortest-path, wikilink, and
  prompt-before-update rules above.
- Autocompletion: `[[`, `![[`, `#` and slash commands, against files, aliases, headings, tags and
  block references.
- Paste/drop: files and clipboard images become attachments at the vault root; a pasted image is
  `Pasted image <local yyyyMMddHHmmss>.png`; multiple dropped files are embedded in order.
  **Copied HTML is not converted** — a web paste writes the clipboard's plain text.
- **Vim key bindings are deleted.**

## 6. Shell

- Frameless window by default with TFT's own title row, window buttons, drag/zoom and window menu;
  `--frame native` asks the desktop for its own frame.
- Left sidebar with **Files** and **Search** tabs. The left ribbon is deleted.
- Main area: horizontal, scrollable tab strip with new-tab and the tab-list menu; tabs can be
  split right/down, dragged, pinned, popped out, and restored from `workspace.json`.
  Stacked tabs are deleted; every group is horizontal.
- The right dock (Outgoing links, Tags, All properties, Outline, File properties) is deleted.
- No bottom-right status bar (file name, Ln/Col, property count are not displayed).
- Files header has sort, auto-reveal, expand/collapse; New note/New folder header buttons are
  deleted.
- Settings is a single window with one scrolling page: **Appearance** (Base color scheme) and
  **Editor** (Spellcheck, Spellcheck languages). General/Hotkeys/Core plugins/Files and links
  tabs and the left Settings sidebar are deleted.
- Notices are transient; the status text remains available to the control socket.

## 7. Views

- **Files** — folder tree, the six sort orders, auto-reveal, keyboard navigation, context menus,
  inline rename, drag/drop, file operations.
- **Search** — Obsidian's query grammar (`path:`, `file:`, `tag:`, `line:`, `section:`, `block:`,
  `task:`, regex, case/explain), grouped results with highlights, Copy search results.
- **Quick switcher** — fuzzy file/alias navigation and “create and open”.
- **Graph** — global/local graph, filters, groups, forces, timelapse; attachments optional.
- **Canvas** — pan/zoom/select/move/resize, cards, note/image/PDF cards, edges with labels,
  groups, alignment, undo; `.canvas` JSON written in Obsidian's bytes.
- **PDF** — hayro rendering, toolbar, sidebar thumbnails, find bar, text selection, spreads,
  embedded `![[x.pdf]]` and `#page=` subpaths.
- **Media** — audio playback and video headers.
- **Math** — RaTeX/KaTeX-compatible display and inline formulas.
- **Mermaid** — diagram layout through limu, painted as a drawing list.
- **Export** — Export to PDF and canvas Export as image, using a gpui-free layout engine,
  krilla and tiny-skia. This feature is optional to the core app.

## 8. Commands, hotkeys, automation

- Obsidian's command ids are the API. `hotkeys.json` overrides default bindings unchanged;
  command ids for deleted features are removed from the inventory.
- `app:open-settings`, `workspace:new-tab`, split/focus/tab commands, file operations, editor
  formatting and table commands, search/switcher/graph/canvas/PDF commands remain.
- The CLI accepts a vault, a file inside a vault, or an `obsidian://` URI; one vault is one window,
  and a second invocation hands the request to the running instance. `--control <socket>` exposes
  a newline-delimited JSON control socket (Unix domain socket on every platform) with methods for
  state, open, mode, keys, mouse, files, commands and quit. Recordings are made through it.
- `crates/control` is the wire client; `apps/desktop/src/control.rs` is the server.

## 9. Testing and compatibility

- `cargo test --workspace` runs unit, corpus and integration tests.
- Parser ratchets: CommonMark's 652 examples and Obsidian's 1000-note corpus.
- Writer ratchet: `TFT_ROUNDTRIP_VAULT=<vault> cargo test -p text --test writer_roundtrip`
  opens every note and writes it back byte-identically, plus targeted CRLF/BOM/frontmatter cases.
- Format fixtures under `crates/*/tests/data` hold bytes Obsidian wrote; do not normalize them.
- The app and libraries build on Linux, Windows and macOS; gpui is pinned to a Zed release commit
  and must be verified on a real GPU before bumping.

## 10. Deliberately out of scope

Bases, Daily notes, Sync, Note composer, Command palette, File recovery, Templates/Unique note
creator, Publish, Web viewer, Audio recorder, Footnotes pane, Slides, Word count, Workspaces,
Bookmarks, Backlinks UI, Random note, stacked tabs, the tab-list chevron menu, the ribbon, the
right dock, the left Settings sidebar, Hotkeys tab, per-plugin options tabs, Vim key bindings,
HTML→Markdown paste, community themes/snippets/fonts, and custom app icons are deleted.
Do not restore them from git history. Obsidian-format identifiers that remain are kept exact.
