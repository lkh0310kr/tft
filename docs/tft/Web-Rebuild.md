---
tags:
  - tft/meta
  - tft/plan
---
# Web rebuild

This is a temporary architecture note for the web rebuild. It supersedes the
prototype `apps/web` state; the prototype is not the reference.

## Why

The first web pass shared only parser/token crates and then built a second app:
its own tabs, selection, file tree, search and settings in `apps/web/src/app.rs`.
That cannot stay identical to GPUI. The web must render the same workspace state
machine, not a lookalike with separate state.

## Target

```text
crates/model      WorkspaceModel, Tab, TabGroup, Layout, ViewMode, LeftTab,
                  HistoryEntry, VaultData, commands and effects
crates/storage    VaultStore trait; LocalStore (desktop), DriveStore (web)
crates/ui-spec    tokens, metrics, component specs
apps/desktop      GPUI renderer + LocalStore + effect runner
apps/web          Leptos renderer + DriveStore + effect runner
```

Rules:

- The desktop is the behavioural reference.
- `apps/web` must not own app state such as tabs, active file, tree or search.
- UI actions go through the same workspace model and command ids.
- I/O is an effect: `Read`, `Write`, `List`, `Rename`, `Delete`.
- Both renderers consume the same model snapshot.
- No second Markdown parser or second editor text model.

## Order

1. **Done:** `crates/model` now owns `WorkspaceModel`, `Tab`, `TabGroup`,
   `Layout`, `ViewMode`, `LeftTab`, `HistoryEntry` and `place_new_tab`.
   Desktop uses the shared `ViewMode`/`LeftTab`/`SplitDirection`/`HistoryEntry`/
   `Tab`; web uses `WorkspaceModel` for tabs, active path, dock tab and modes.
2. **Done:** `VaultData` is a canonical node graph: `NodeId`, `BackendObjectId`,
   `parent + name`, derived `children`/paths/conflicts/cache. `apply_scan` keeps
   backend identities; same-parent duplicate names are `Ambiguous`, never merged.
   `Vault::apply_snapshot` is the desktop adapter and `VaultState::verify_data`
   compares it with the path vault.
3. **Done for the shell:** web builds `VaultData` from Drive folder IDs, renders
   `FileTree::from_vault`, and keys tabs, open, save, rename, delete, search and
   the switcher by `NodeId`. Paths remain display projections.
4. **Next:** wire `VaultEffect`/`VaultEvent` runners for Drive and local I/O, then
   move desktop `Workspace.current`/history and `VaultState.cache/texts` onto
   `NodeId`.
5. Port the shell module by module against the GPUI reference.
6. Live Preview, history, split/drag, graph/canvas/PDF.
7. Acceptance gates below.

## Acceptance

- Same action sequence produces the same model snapshot on desktop and web.
- Tree rows, order, collapse and counts match.
- Tabs, active group, split and history match.
- Reading tree and editor bytes match.
- Same command id dispatches the same action.
- UI measurements/screenshots are compared per view.

## Status

The Drive path now shares the node graph: folder IDs survive indexing, empty
folders are rows, and duplicate names render as conflict rows that refuse to open
until renamed. Tabs survive a rename because `Tab.node` is the identity. The
remaining prototype-only state is the `files`/`entries` projection used for file
kind and Drive request IDs, and the path-based desktop workspace.
