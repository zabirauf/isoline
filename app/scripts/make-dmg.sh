#!/bin/sh
# Pack the built Isoline.app into a compressed DMG with hdiutil. Tauri's own DMG step lays out
# the window with a Finder AppleScript, which needs automation permission and times out
# without it; this produces the same installable image without that step.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
# Optional first argument: a Rust target triple, for apps built with `tauri build --target`.
triple=${1:-}
if [ -n "$triple" ]; then rel="$here/target/$triple/release"; else rel="$here/target/release"; fi
app="$rel/bundle/macos/Isoline.app"
out="$rel/bundle/dmg"
[ -d "$app" ] || { echo "make-dmg: build the app first (npm run build:app)" >&2; exit 1; }
version=$(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$here/app/src-tauri/tauri.conf.json" | head -1)
case "$triple" in
  aarch64-*) arch=aarch64 ;;
  x86_64-*) arch=x64 ;;
  *) arch=$(uname -m) ;;
esac
mkdir -p "$out"
dmg="$out/Isoline_${version}_${arch}.dmg"
hdiutil create -volname Isoline -srcfolder "$app" -ov -format UDZO "$dmg" >/dev/null
echo "$dmg"
