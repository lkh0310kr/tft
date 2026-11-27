#!/bin/sh
# Apple Color Emoji's sbix PNGs need the hardened runtime flag on Apple
# Silicon: without it CoreText crashes in CopyEmojiImage with SIGBUS
# (EXC_ARM_DA_ALIGN), taking the app down whenever an emoji glyph is
# painted. Cargo linkers sign their binaries ad hoc, which lacks the flag,
# so add it before every binary `cargo run` and `cargo test` starts.
#
# A signing failure must not stop the run: the binary still works until an
# emoji is drawn, and machines without codesign should keep building.
if [ "$(uname -s)" = Darwin ] && command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - --options runtime --timestamp=none "$1" >/dev/null 2>&1 || true
fi
exec "$@"
