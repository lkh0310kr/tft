# TFT

TFT is a native Markdown knowledge base written in Rust on [GPUI](https://gpui.rs). It stores
notes and app settings in the same vault folder and does not create a second copy of note files.

> Keep a backup of any vault you open with it. Editing is newer code against files you cannot get
> back.

## What it does

- **Vaults** are folders. TFT scans them, watches them, and preserves line endings, byte-order
  marks, JSON key order and unknown config keys. The vault is the source of truth.
- **Reading**: CommonMark, GitHub Markdown extensions, wikilinks and embeds, callouts,
  properties, tables, math, Mermaid diagrams, code highlighting, PDF pages and media. One
  Markdown parser feeds reading, editing, the metadata cache and link rewriting.
- **Writing**: Live Preview and Source modes, find and replace, multiple cursors, auto-pairing,
  smart lists, table editing with row/column commands and handles, typed properties, block
  references, slash commands and autocompletion for `[[`, `![[` and `#`.
- **Shell**: a frameless window with Files and Search, a horizontally scrolling tab strip,
  split tab groups, draggable tabs, popout windows, quick switcher and graph.
- **Canvas and PDF**: `.canvas` pan/zoom/edit/undo in JSON Canvas format; PDF viewing,
  thumbnails, search and text selection, including embedded `![[x.pdf]]`.
- **Export**: Export to PDF and canvas Export as image.
- **Automation**: command IDs and `hotkeys.json` bindings, a CLI that hands a second
  invocation to the running window, and an optional local control socket for tests and recordings.

Settings is deliberately small: Appearance has Light / Dark / System, and Editor has spellcheck
and its languages. Core plugins are always on; their separate settings pages and the old tabbed
Settings sidebar are gone. See [docs/Specification.md](docs/Specification.md) for the
normative behavior and the deliberately deleted features.

## Running it

The vault argument may be a folder, a file inside a vault, or an `tft://` URI:

```sh
cargo run --release -p desktop -- /path/to/vault
cargo run --release -p desktop -- /path/to/vault/Note.md
cargo run --release -p desktop -- "/path/to/vault" --frame native
```

`tft --help` prints every flag. One vault is one window; a second invocation hands the file, note
or URI to the open window and raises it. `--control <socket>` exposes the running app on a local
Unix-domain socket speaking newline-delimited JSON for tests and recordings.

## Web apps (Google Drive)

The active Web target is `apps/web`, a GPUI WebAssembly application. The previous
Leptos shell is preserved at `apps/web-leptos` in its own Cargo workspace.
GPUI Web build and browser checks are documented in [apps/web/README.md](apps/web/README.md).

The setup notes below apply to the preserved Leptos deployment unless stated otherwise.

### First setup

1. Create a Google Cloud project and enable **Google Drive API** and **Google
   Picker API** (APIs & Services → Library).
2. Configure the OAuth consent screen (External). Add
   `https://www.googleapis.com/auth/drive.file` under Data Access and add your
   account as a test user.
3. Create **Credentials → OAuth client ID → Web application**.
4. Add these **Authorized JavaScript origins**:
   - `http://127.0.0.1:8080`
   - `https://tft-web-sage.vercel.app` (production)
5. Copy `apps/web/config.example.js` to `apps/web/config.js` and fill in the
   client ID. The API key is optional for Picker; leave it empty first.
6. Install Trunk once, then serve:

   ```sh
   cargo install trunk --locked
   cd apps/web
   trunk serve
   ```

7. Open `http://127.0.0.1:8080`, sign in, then **Pick vault**. If Picker is not
   available, paste a Drive folder ID into the fallback field or use **Create
   test vault**.

`config.js` is gitignored because it names one Google project. It holds public
identifiers only: the OAuth client ID and, if needed, a browser API key. Never
put the OAuth **client secret**, a refresh token or a user access token in this
file, in any other frontend file, or in the repository. If one is exposed,
revoke it in Google Cloud Console.

Detailed Picker troubleshooting (401, `API key not valid`, `access_denied`) is in
[apps/web/README.md](apps/web/README.md).

### Deploying the web shell

The deployment target is Vercel, project `tft-web`, production origin
`https://tft-web-sage.vercel.app`.

```powershell
cd apps/web
.\deploy.ps1                 # builds with Trunk, blanks the API key, deploys
.\deploy.ps1 -WithApiKey     # only for a key restricted to the Vercel origin
```

Before deploying, the Vercel origin must be in the OAuth client's Authorized
JavaScript origins. `vercel.json` supplies the CSP and security headers. Only
public identifiers are shipped: no client secret, no token, no vault data.
Files stay in the user's Drive.

## Building and testing

TFT builds on Linux, Windows and macOS. gpui is pinned to a Zed release commit; build it on a real
GPU before bumping the pin.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo wasm-check
```

`TFT_ROUNDTRIP_VAULT=<vault> cargo test -p text --test writer_roundtrip` opens every note in a
real vault and asserts that writing it back changes no bytes.

## Layout

```
apps/desktop        GPUI application (package desktop; binary tft)
apps/web            GPUI WebAssembly app shell backed by Google Drive
apps/web-leptos     preserved Leptos implementation (separate workspace)
crates/app          shared app state/use cases and wasm-safe services
crates/domain       VaultData graph, storage identities and invariants
crates/vault        vault scanning, file kinds, .tft reads/writes
crates/markdown     CommonMark + GFM + TFT Markdown extensions, one spanned tree
crates/cache        CachedMetadata and the link index/resolver
crates/formats      on-disk formats: configs, canvas, properties
crates/commands     command ids, registry, default hotkeys
crates/text         editor text model: buffer, selections, undo, edit plans
crates/suggest      completion triggers, candidates, accepted-link text
crates/search       query grammar, matcher, fuzzy search
crates/canvas       canvas viewport, geometry, hit tests, edits
crates/pdf          PDF pages, outline, text, #page= subpath
crates/math         TeX typeset on RaTeX (KaTeX-compatible)
crates/mermaid      Mermaid layout through limu
crates/highlight    Prism tokenizer and language grammars
crates/export       Export to PDF (krilla) and Export as image (tiny-skia)
crates/audio        audio playback and video headers
crates/paste        attachment naming, text/uri-list decoding
crates/spell        Hunspell spellcheck and dictionaries
crates/control      control-socket protocol and client
crates/cli          flags, file/URI arguments, one-window handover
crates/bench        synthetic vault generator and index timer
```

## Compatibility and security

Vault and format compatibility come first: no conversion, no data loss. The Markdown parser is
held to CommonMark's 652 examples and the checked-in rendering corpus; the writer is held to
byte-identical round trips. Theme compatibility is the two built-in palettes only, and there is
no community-plugin runtime.

The app writes only inside the vault (apart from moving a deleted file to the system trash),
never executes vault content, and its control socket is local and owner-only.

## License

MIT. See [LICENSE](LICENSE).
