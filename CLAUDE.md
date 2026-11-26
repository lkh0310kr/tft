# TFT

Native Rust/GPUI clone of Obsidian's desktop app, targeting byte-level vault compatibility.

## Ground rules

- The vault folder is the only source of truth. Never write a file TFT did not change; preserve
  line endings, BOMs, and unknown JSON/YAML keys (`#[serde(flatten)] extra` everywhere).
- One Markdown parser (`markdown`) feeds the editor, reading view, cache, and link rewriting.
  Do not add a second tokenizer for any of those.
- Formats and behaviors are specified in `docs/tft/Specification.md`. When the spec and Obsidian disagree, test
  against a real vault and fix the spec.
- Command ids, config keys, and file names must match Obsidian's exactly (`commands/data/commands.txt`,
  `formats/src/config.rs`). Add new ones only with the observed string.
- Clean room: implement from the spec and public docs. Do not paste code, CSS, or icons from
  Obsidian's bundle.

## Layout

```
apps/desktop           GPUI app (bin): workspace shell, Files view, reading view, source editor
apps/web                GPUI WebAssembly app shell backed by Google Drive
apps/web-leptos         preserved Leptos implementation (separate workspace)
assets                 icons and fonts shared by the desktop app and the future web app
crates/app       shared app state/use cases and wasm-safe services
crates/domain    VaultData graph, storage identities and invariants
crates/vault     folder scanning, .obsidian config
crates/markdown  OFM primitives
crates/cache     CachedMetadata, LinkResolver
crates/formats   FileKind, config types, canvas, base
crates/commands  command registry, default hotkeys
crates/cli       the command line: flags and help, the file argument, the one-window-per-vault handover (no gpui)
crates/search    search query grammar and matcher, fuzzy search, Excluded files
crates/text      editor text model: buffer, selections, undo, edit plans (no gpui)
crates/suggest   autocompletion: triggers, candidates, the link an accepted row writes (no gpui)
crates/canvas    canvas model: viewport math, edge geometry, hit tests, selection, edits with undo (no gpui)
crates/pdf       PDF rendering on hayro: pages to RGBA, outline, title, page text, the #page= subpath (no gpui)
crates/math      TeX math typeset on RaTeX (KaTeX-compatible): a drawing list in em units plus the KaTeX fonts (no gpui)
crates/highlight code highlighting: Prism's tokenizer and grammars ported, and the --code-* each token paints from (no gpui)
crates/export    Export to PDF and Export as image: block layout and pagination, the PDF (krilla), the canvas PNG (tiny-skia) (no gpui)
crates/audio     the media player (symphonia + opus-decoder, cpal out) and a video file's header (no gpui)
crates/paste     what a paste or a drop writes: attachment names, a uri-list's paths (no gpui)
crates/spell     spellcheck: Hunspell dictionaries on spellbook (SCOWL en_US bundled, others from the system), the words markdown leaves to check, the custom dictionary (no gpui)
crates/mermaid   Mermaid diagrams laid out as Obsidian's Pele lays them out (limu), into a drawing list (no gpui)
crates/control   the control socket's wire types, method table and a blocking client (no gpui)
crates/bench     spike benchmarks: synthetic vault generator, index timer (bin)
docs/tft             Obsidian vault: the concise project specification (dogfooded by TFT)
tools/prism-port        writes highlight's generated grammars and its Prism corpus from Prism 1.29 (node); change it and re-run, never hand-edit the output
tools/plugin-spike      phase 5's runtime spike: WIT components on wasmtime against extism, measured (OUTSIDE the cargo workspace)
tools/writer-gate       the Writer gate's baselines and diffs: `gate.sh check`, `gate.sh baseline`, `gate.sh status`
tools/obsidian-probe    the probe scripts; `reference/observed-behavior.md` is the raw observation record behind the specification
```

## Commands

On NixOS run these inside `nix-shell` (`shell.nix` supplies gpui's X11, Wayland, and Vulkan libraries).
`CARGO_TARGET_DIR` defaults to the repo's `target/`. The repo sits on an SMB share that a Linux
machine also builds in.
On the Mac, `build.build-dir` in `~/.cargo/config.toml` keeps intermediate artifacts on the local disk:
smbfs has no fcntl locks, which incremental compilation needs. If the share is ever mounted with the
quarantine flag again, binaries written there make Gatekeeper prompt; point `CARGO_TARGET_DIR` at a
local directory then.
gpui is a git dependency pinned to a Zed release commit (see the note in `Cargo.toml`); the crates.io
release is stale and crashes on NVIDIA. Bump the pin on its own, one release at a time, and run the
app on a real GPU before committing.

```
cargo test --workspace            # all library tests (fast)
cargo wasm-check                  # pure crates for wasm32-unknown-unknown
TFT_ROUNDTRIP_VAULT=<vault> cargo test -p text --test writer_roundtrip   # Writer gate round trips, over a real vault too
cargo check -p desktop             # app, pulls gpui (slow first time)
cargo run --release -p desktop -- <vault> [--note "Name#Heading"]   # open a vault (release: the editor reparses on every edit)
cargo run --release -p desktop -- --help                            # every flag, written from cli's own table
cargo run --release -p desktop -- <vault>/Note.md --new-window      # a file argument; --new-window skips the handover a second run makes
cargo run --release -p desktop -- <vault> --frame native   # the desktop's title bar instead of the hidden frame (TFT_FRAME=native does the same)
cargo run --release -p desktop -- <vault> --synthetic 50000 --stress   # editor spike: self-driven edits, prints timings
cargo run --release -p desktop -- <vault> --note X --source --type '[[al' [--accept enter]  # open the autocompletion popup (and press its keys) for a screenshot
cargo run --release -p bench -- generate <dir>   # synthetic 20k-note vault
cargo run --release -p bench -- index <vault>    # time scan/read/index/resolve, check the 2 s gate
cargo fmt --all && cargo clippy --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings   # clippy as CI ran it: current stable, warnings denied
```

## Conventions

- Vault-relative paths use `/` and keep original case; comparisons are case-insensitive.
- Positions are zero-based line/column plus byte offset, like Obsidian's `Pos`.
- Tests that need a vault build one in a `tempfile::tempdir()`; never depend on the developer's vault.
- `crates/markdown/tests/corpus` holds Obsidian's own rendering of a 1000-note vault; the
  corpus test and the CommonMark suite are ratchets. When Obsidian and the parser disagree, add a
  probe note to `tests/corpus/vault/edge`, re-render with the plugin in that vault, and match the
  observed behavior (list it in `docs/tft/Specification.md`).
- `docs/tft/` is an Obsidian vault: write notes with wikilinks and frontmatter, and keep
  `.obsidian/` committed except `workspace.json`.
