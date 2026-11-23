# TFT handoff

TFT is a native Rust/GPUI clone of Obsidian's desktop app with byte-level vault
compatibility. The same workspace logic is being shared with a Leptos web shell
backed by Google Drive (`drive.file`).

- Repository: `lkh0310kr/tft`
- Desktop app: `apps/desktop` (binary `tft`)
- Web app: `apps/web` (separate Leptos CSR workspace, deployed on Vercel)
- Spec: `docs/tft/Specification.md`
- Web rebuild plan and status: `docs/tft/Web-Rebuild.md`

## Architecture now

```
crates/model      WorkspaceModel, Tab, TabGroup, Layout, ViewMode, LeftTab,
                  HistoryEntry, VaultData (canonical node graph), effects/events
crates/core       framework-neutral UI logic: FileTree, design tokens, vault plans
crates/vault      local folder scanning, .obsidian config, path operations,
                  Vault::apply_snapshot (local adapter into VaultData)
crates/markdown   the one Markdown parser used by editor, reading, cache and links
apps/desktop      GPUI renderer; still path-based internally, with VaultData sync
apps/web          Leptos renderer; Drive index/read/write, NodeId-first UI
```

### VaultData invariants

- `nodes` is canonical. `children`, `paths`, `by_path`, `names`, conflicts and
  the metadata cache are derived indexes.
- `NodeId(u64)` is runtime identity. `BackendObjectId::{Local,Drive}` is
  persistent storage identity. `BackendInstanceId` identifies one vault.
- `parent + name` is the hierarchy. `path` is derived for display and lookup.
- Same-parent duplicate names (case-insensitive, file or folder) are conflicts:
  both nodes are kept, `path` is `None`, lookup is `Ambiguous`, descendants are
  ambiguous too. TFT never auto-renames or adds a suffix.
- Same name under different parents is valid. Empty folders are nodes.
- Tabs, history, selection, search results and file operations use `NodeId`;
  paths are projections only.

### Desktop migration state

`VaultState` keeps the path-based `Vault`/cache/texts as the behavioural source,
and carries a synchronized `VaultData`:

- `load` calls `verify_data()`: file/folder projections and graph invariants must
  agree or the vault refuses to open.
- structural changes (`create`, `trash`, `apply`) resync the graph.
- `rename` calls `rebase_local_prefix` first so renamed nodes keep their NodeId.

Desktop `Workspace.current`, tab strips and history are still path-based. Moving
them to `NodeId` is the remaining desktop step.

### Web state

- `index_vault` walks Drive folders BFS, preserves folder IDs, includes empty
  folders and builds `VaultData` with `apply_scan`.
- `FileTree::from_vault` renders the graph; conflict rows are marked and refuse
  to open.
- `IndexedFile` holds `NodeId`, path, kind, Drive ID and ETag. Open/save/rename/
  delete/search/switcher use `NodeId`.
- Note text is loaded in the background with `drive::load_markdown` and inserted
  through `VaultData::set_text`.

## Build and test

Run inside `nix-shell` on NixOS; on Windows/macOS a normal Rust toolchain works.

```sh
cargo test --workspace
cargo wasm-check
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p desktop
cargo run --release -p desktop -- <vault>

cd apps/web
trunk serve                      # http://127.0.0.1:8080
cargo clippy --target wasm32-unknown-unknown -- -D warnings
./deploy.ps1                     # build + deploy to Vercel (PowerShell)
```

The web production URL is `https://tft-web-sage.vercel.app`.

## Configuration and security

- `apps/web/config.js` is gitignored. It holds the browser OAuth client ID and,
  locally, an API key. Never put a client secret in it or in this repository.
- `deploy.ps1` blanks `apiKey` in the deployed bundle by default. Pass
  `-WithApiKey` only when the Picker genuinely needs a restricted browser key.
- Add both `http://127.0.0.1:8080` and the Vercel origin to the OAuth client's
  Authorized JavaScript origins. The production origin currently needs
  `https://tft-web-sage.vercel.app` registered.
- OAuth scope is `drive.file` plus Picker; TFT only sees files the user picks.
- No client secret, refresh token or user token belongs in the repo. If one is
  pasted into a chat or log, revoke it in Google Cloud Console.

## Remaining work

1. Wire `VaultEffect`/`VaultEvent` runners: local filesystem for desktop and a
   prepared Drive operation for web. The model, events and `apply_event` exist.
2. Move desktop `Workspace.current`, tabs, history, tree focus and rename
   follow-through to `NodeId`.
3. Port the desktop shell module by module (dock, tab strip, split, context
   menus, settings) so the web renderer owns no app-specific state.
4. Verify against the real PKMS Drive vault in a browser: file/folder/conflict
   counts, expand state, rename, tab identity, search and reading.
5. Live Preview, history navigation and split/drag parity, then the graph,
   canvas and PDF views.
6. Update the acceptance matrix in `docs/tft/Web-Rebuild.md` as each gate passes.

## Known caveats

- The desktop local backend token is the file path. TFT renames rebase it, but a
  rename performed outside TFT appears as a new node (new NodeId).
- Conflicted paths are intentionally unopenable/unlinkable until the user
  renames one node. This is a data-safety choice, not a loading bug.
- `verify_data()` is a hard gate on desktop load. If a new vault shape trips it,
  fix `apply_snapshot`/`verify_data` rather than bypassing the check.
- The web bundle contains no API key by default; a Picker failure after deploy is
  usually a missing authorized origin, not missing code.
