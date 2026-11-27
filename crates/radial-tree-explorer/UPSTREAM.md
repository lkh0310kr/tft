The radial focus tree engine and GPUI renderer were brought in from
`https://github.com/lkh0310kr/workspace-rust`, commit `e53206e`.

This crate keeps the UI-independent state and transition engine in `engine` and
the GPUI renderer in `tree_view`. It is wired into TFT as an optional
folder-tree view; the existing link graph remains separate. The bundled
Montserrat font license is in `assets/fonts/OFL.txt`.
