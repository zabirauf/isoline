#!/bin/sh
# Pack the built Isoline.app into a compressed DMG with hdiutil. Tauri's own DMG step lays out
# the window with a Finder AppleScript, which needs automation permission and times out
# without it; this produces the same installable image without that step.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
app="$here/target/release/bundle/macos/Isoline.app"
out="$here/target/release/bundle/dmg"
[ -d "$app" ] || { echo "make-dmg: build the app first (npm run build:app)" >&2; exit 1; }
version=$(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$here/app/src-tauri/tauri.conf.json" | head -1)
arch=$(uname -m)
mkdir -p "$out"
dmg="$out/Isoline_${version}_${arch}.dmg"
hdiutil create -volname Isoline -srcfolder "$app" -ov -format UDZO "$dmg" >/dev/null
echo "$dmg"
