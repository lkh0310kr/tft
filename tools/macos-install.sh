#!/usr/bin/env bash
# Build TFT.app (release), install it over the previous copy, and relaunch.
#
#   tools/macos-install.sh                 # release build -> /Applications/TFT.app -> open
#   tools/macos-install.sh --no-launch     # install without opening it again
#   TFT_APP_DIR="$HOME/Applications" tools/macos-install.sh
#   TFT_BUNDLE_ID=com.example.tft tools/macos-install.sh
#   TFT_KEEP_RUNNING=1 tools/macos-install.sh   # do not close other running TFTs
#
# The binary is ad-hoc signed with the hardened runtime. Apple Color Emoji is
# painted through CoreText, which SIGBUSes on Apple Silicon when that flag is
# missing (see tools/macos-run.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="TFT"
BIN_NAME="tft"
BUNDLE_ID="${TFT_BUNDLE_ID:-com.github.lkh0310kr.tft}"
APP_DIR="${TFT_APP_DIR:-/Applications}"
APP_BUNDLE="$APP_DIR/$APP_NAME.app"
STAGE="$ROOT/target/macos/$APP_NAME.app"
LAUNCH=1

for arg in "$@"; do
  case "$arg" in
    --no-launch) LAUNCH=0 ;;
    --help|-h)
      sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      printf 'Unknown argument: %s\n' "$arg" >&2
      exit 2
      ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  printf 'macos-install.sh only runs on macOS.\n' >&2
  exit 1
fi

VERSION="$(awk '
  /^\[workspace.package\]/ { section = 1; next }
  /^\[/ { section = 0 }
  section && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
' "$ROOT/Cargo.toml")"
if [ -z "$VERSION" ]; then
  printf 'Could not read the workspace version from Cargo.toml.\n' >&2
  exit 1
fi
BUILD_NUMBER="$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || date +%Y%m%d%H%M)"

if [ "${TFT_SKIP_BUILD:-0}" = "1" ]; then
  TARGET_DIR="$(cargo metadata --format-version 1 --no-deps --manifest-path "$ROOT/Cargo.toml" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  BIN="$TARGET_DIR/release/$BIN_NAME"
else
  printf '==> Building %s %s (release)\n' "$APP_NAME" "$VERSION"
  BUILD_JSON="$(mktemp -t tft-build.XXXXXX)"
  trap 'rm -f "$BUILD_JSON"' EXIT
  cargo build --manifest-path "$ROOT/Cargo.toml" --release -p desktop --bin "$BIN_NAME" \
    --message-format=json-render-diagnostics >"$BUILD_JSON"
  BIN="$(python3 - "$BUILD_JSON" <<'PY'
import json
import sys

path = ""
for line in open(sys.argv[1], encoding="utf-8"):
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    if (
        message.get("reason") == "compiler-artifact"
        and message.get("target", {}).get("name") == "tft"
        and message.get("executable")
    ):
        path = message["executable"]
print(path)
PY
)"
fi
if [ ! -x "$BIN" ]; then
  printf 'Release binary not found: %s\n' "$BIN" >&2
  exit 1
fi

printf '==> Assembling %s\n' "$STAGE"
rm -rf "$STAGE"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$BIN" "$STAGE/Contents/MacOS/$BIN_NAME"

# Icon: render the repository logo into a square 1024 canvas and let iconutil
# emit the .icns. Missing image tools only cost the Dock icon.
if [ -f "$ROOT/assets/logo.svg" ] \
  && command -v rsvg-convert >/dev/null 2>&1 \
  && command -v sips >/dev/null 2>&1 \
  && command -v iconutil >/dev/null 2>&1; then
  ICONSET="$ROOT/target/macos/AppIcon.iconset"
  MASTER="$ROOT/target/macos/AppIcon-1024.png"
  rm -rf "$ICONSET"
  mkdir -p "$ICONSET"
  # viewBox is 603.3 x 430.9; 860 px wide leaves a 82 px side margin and a
  # 614 px high image centered in the 1024 square.
  rsvg-convert -w 860 --page-width 1024 --page-height 1024 --left 82 --top 205 \
    -o "$MASTER" "$ROOT/assets/logo.svg"
  for size in 16 32 64 128 256 512; do
    sips -z "$size" "$size" "$MASTER" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$MASTER" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$STAGE/Contents/Resources/AppIcon.icns"
fi

cat >"$STAGE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$BIN_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSDocumentsFolderUsageDescription</key><string>TFT opens the Markdown vault you choose.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>TFT opens the Markdown vault you choose.</string>
  <key>NSDesktopFolderUsageDescription</key><string>TFT opens the Markdown vault you choose.</string>
</dict>
</plist>
PLIST
plutil -lint "$STAGE/Contents/Info.plist" >/dev/null

printf '==> Signing (ad-hoc, hardened runtime)\n'
codesign --force --sign - --options runtime --timestamp=none "$STAGE" >/dev/null
codesign --verify --deep --strict "$STAGE" >/dev/null
SIGN_INFO="$(codesign -dv --verbose=2 "$STAGE" 2>&1 || true)"
case "$SIGN_INFO" in
  *flags=*runtime*) ;;
  *)
    printf 'Signed app is missing the hardened runtime flag:\n%s\n' "$SIGN_INFO" >&2
    exit 1
    ;;
esac

mkdir -p "$APP_DIR"
if [ ! -w "$APP_DIR" ]; then
  printf 'Cannot write to %s. Set TFT_APP_DIR to a writable Applications folder.\n' "$APP_DIR" >&2
  exit 1
fi

if [ -d "$APP_BUNDLE" ]; then
  printf '==> Quitting the running copy\n'
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 25); do
    if ! pgrep -f "$APP_BUNDLE/Contents/MacOS/$BIN_NAME" >/dev/null 2>&1; then
      break
    fi
    sleep 0.2
  done
  pkill -f "$APP_BUNDLE/Contents/MacOS/$BIN_NAME" >/dev/null 2>&1 || true
  sleep 0.5
  rm -rf "$APP_BUNDLE"
fi

printf '==> Installing %s\n' "$APP_BUNDLE"
ditto "$STAGE" "$APP_BUNDLE"
xattr -dr com.apple.quarantine "$APP_BUNDLE" >/dev/null 2>&1 || true

# A second TFT (a dev build, for example) holding the last vault answers the
# new launch's handover socket and the updated app exits with no window.
# Close those instances so the bundle just installed is the one that opens.
if [ "${TFT_KEEP_RUNNING:-0}" != "1" ]; then
  for pid in $(pgrep -x "$BIN_NAME" 2>/dev/null || true); do
    printf '==> Closing the other running TFT (pid %s)\n' "$pid"
    kill -TERM "$pid" >/dev/null 2>&1 || true
  done
  for _ in $(seq 1 20); do
    if ! pgrep -x "$BIN_NAME" >/dev/null 2>&1; then
      break
    fi
    sleep 0.1
  done
  pkill -x "$BIN_NAME" >/dev/null 2>&1 || true
fi

if [ "$LAUNCH" = "1" ]; then
  printf '==> Launching\n'
  # --new-window skips the single-instance handover, so the launch cannot be
  # taken over by an older TFT (for example a dev build) still running.
  open -n "$APP_BUNDLE" --args --new-window
fi
printf 'Installed %s %s (%s) at %s\n' "$APP_NAME" "$VERSION" "$BUILD_NUMBER" "$APP_BUNDLE"
