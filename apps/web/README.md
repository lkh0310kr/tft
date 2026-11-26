# TFT GPUI Web application

The normal WASM build mounts `DriveApp` from this Web app. It collects public
Google OAuth and Picker configuration, starts Google sign-in and folder picking
from direct user clicks, indexes the chosen Drive Vault into `VaultData`, loads
Markdown source, and exposes a Save to Drive action. The editor pane uses
the local `WebWorkspace`, production `Editor`, shared `Theme`,
`FileTree`, and `WorkspaceModel`.

The `browser-test` feature mounts an in-memory fixture instead of making Google
requests. `test_drive_shell.py` verifies the normal app screen, the production
GPUI `TextInput`, browser-local config persistence, reload behavior and console
errors. `test_drive_flow.py` stubs GIS, Picker, and Drive REST at the browser
network boundary and exercises sign-in, folder selection, metadata/list/media
reads, Markdown editing, and the Drive upload PATCH including its exact source
body. `test_browser.py` verifies shared workspace and text editing actions,
including reading preview/source switching and closing the active shared-model
tab.
The flow test does not use Google credentials or write a real Drive file; those
checks still require a configured OAuth client, Picker API key, and test Vault.

The pinned GPUI revision has a Web application lifetime bug: `Application::run`
returns while `WebPlatform::run` is still awaiting startup, then releases the
last strong `AppCell` reference. RAF/ResizeObserver/input callbacks subsequently
invoke dropped closures. The WASM-only patch is recorded in
`patches/gpui-app-run-wasm-lifetime.patch`. `build-web.sh` and `check-wasm.sh`
verify the exact pinned checkout and apply that patch idempotently before use.
This remains a local dependency patch; production adoption still needs a
maintained GPUI revision or fork.

## Build and browser checks

```sh
rustup target add wasm32-unknown-unknown
apps/web/check-wasm.sh
apps/web/build-web.sh
apps/web/serve-web.sh 8088
TRUNK_BIN=/path/to/trunk python3 apps/web/tests/test_browser.py
TRUNK_BIN=/path/to/trunk python3 apps/web/tests/test_drive_shell.py
TRUNK_BIN=/path/to/trunk python3 apps/web/tests/test_drive_flow.py
```

All repository-owned Web builds go through `build-web.sh`; it applies the
revision-checked GPUI patch and sets the required WASM bootstrap environment.
`serve-web.sh` builds and serves the same `dist` bundle. Set `TRUNK_BIN` only
when Trunk is not on `PATH`.

The test script builds with `browser-test`, serves the resulting bundle and
runs Chrome/Playwright checks for the shared workspace, file-to-tab flow,
buffer preservation across tab switches, Korean glyphs, double-click and drag
selection, Cmd/Option cursor movement, source selection, clipboard copy/paste,
undo/redo, ASCII typing, synthetic Hangul composition and page/console errors.
It saves `/tmp/tft-shared-editor.png`. The composition case sends synthetic
browser events; it does not validate a real Korean IME. Physical iOS Safari/PWA
testing remains outstanding.

`RUSTC_BOOTSTRAP=1` is needed because this GPUI dependency graph uses a nightly
feature in `wasm_thread`. Drive credentials are entered in the GPUI setup
screen and saved in the current browser's local storage. The OAuth JavaScript
origin must be allowed in Google Cloud, with Drive API and Picker API enabled.
To build and serve the normal app, run `apps/web/serve-web.sh 8088`.
