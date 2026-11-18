#!/bin/sh
# Install or update Isoline from GitHub releases on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/zabirauf/isoline/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/zabirauf/isoline/main/scripts/install.sh | sh -s -- --stable --cli
#
# --stable   the latest v* release instead of the rolling nightly built from main
# --cli      also install iso-cli into ~/.local/bin
#
# macOS: Isoline.app goes to /Applications (or ~/Applications when that isn't writable).
# Linux: the AppImage goes to ~/.local/bin/isoline.
# Every download is checked against the release's SHA256SUMS.txt.
# ISOLINE_APP_DIR and ISOLINE_BIN_DIR override the install folders.
set -eu

REPO=zabirauf/isoline
channel=nightly
cli=0
for arg in "$@"; do
  case "$arg" in
    --stable) channel=stable ;;
    --nightly) channel=nightly ;;
    --cli) cli=1 ;;
    -h|--help) sed -n '2,13p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
die() { printf 'isoline install: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "needs $1"; }
need curl
need uname

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Darwin/arm64) app_suffix=_macos-arm64.app.zip; cli_suffix=_macos-arm64.tar.gz ;;
  Darwin/x86_64) app_suffix=_macos-x64.app.zip; cli_suffix=_macos-x64.tar.gz ;;
  Linux/x86_64) app_suffix=_amd64.AppImage; cli_suffix=_linux-x64.tar.gz ;;
  Linux/aarch64|Linux/arm64) app_suffix=_aarch64.AppImage; cli_suffix=_linux-arm64.tar.gz ;;
  *) die "no build for $os/$arch; on Windows, download from https://github.com/$REPO/releases" ;;
esac

if [ "$channel" = nightly ]; then
  api=https://api.github.com/repos/$REPO/releases/tags/nightly
else
  api=https://api.github.com/repos/$REPO/releases/latest
fi
release=$(curl -fsSL -H 'Accept: application/vnd.github+json' "$api") || die "could not read $api"
title=$(printf '%s' "$release" | sed -n 's/^  "name": "\(.*\)",$/\1/p' | head -1)
title=${title:-Isoline $channel}
bin_dir=${ISOLINE_BIN_DIR:-$HOME/.local/bin}
urls=$(printf '%s' "$release" | grep -o '"browser_download_url": *"[^"]*"' | sed 's/.*"\(https[^"]*\)"/\1/')
pick() { printf '%s\n' "$urls" | grep -- "$1\$" | head -1; }
app_url=$(pick "$app_suffix")
sums_url=$(pick "/SHA256SUMS.txt")
[ -n "$app_url" ] || die "$title has no file ending in $app_suffix"
[ -n "$sums_url" ] || die "$title has no SHA256SUMS.txt"

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t isoline)
trap 'rm -rf "$tmp"' EXIT INT TERM

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
fetch() {
  name=${1##*/}
  say "Downloading $name"
  curl -fL --progress-bar -o "$tmp/$name" "$1" || die "download failed: $1"
  want=$(grep " \*\{0,1\}$name\$" "$tmp/SHA256SUMS.txt" | cut -d' ' -f1)
  [ -n "$want" ] || die "$name is not in SHA256SUMS.txt"
  [ "$(sha256 "$tmp/$name")" = "$want" ] || die "checksum mismatch for $name"
}
curl -fsSL -o "$tmp/SHA256SUMS.txt" "$sums_url" || die "could not download SHA256SUMS.txt"

say "Installing $title"
fetch "$app_url"
app_file=$tmp/${app_url##*/}

case "$os" in
  Darwin)
    need ditto
    ditto -x -k "$app_file" "$tmp/unpacked"
    [ -d "$tmp/unpacked/Isoline.app" ] || die "the archive has no Isoline.app"
    if [ -n "${ISOLINE_APP_DIR:-}" ]; then app_dir=$ISOLINE_APP_DIR
    elif [ -w /Applications ]; then app_dir=/Applications
    else app_dir=$HOME/Applications; fi
    mkdir -p "$app_dir"
    dest=$app_dir/Isoline.app
    if [ -z "${ISOLINE_APP_DIR:-}" ] && pgrep -x isoline >/dev/null 2>&1; then
      say "Quitting the running Isoline"
      osascript -e 'quit app "Isoline"' >/dev/null 2>&1 || true
      i=0; while pgrep -x isoline >/dev/null 2>&1 && [ $i -lt 20 ]; do sleep 0.5; i=$((i + 1)); done
    fi
    # Replace the bundle's contents rather than the folder: macOS may refuse to delete an app
    # folder in /Applications while still allowing its contents to be replaced.
    rm -rf "$dest/Contents" 2>/dev/null || true
    ditto "$tmp/unpacked/Isoline.app" "$dest"
    xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true
    say "Installed $dest"
    ;;
  Linux)
    mkdir -p "$bin_dir"
    install -m 755 "$app_file" "$bin_dir/isoline"
    say "Installed $bin_dir/isoline"
    ;;
esac

if [ "$cli" = 1 ]; then
  cli_url=$(pick "$cli_suffix")
  [ -n "$cli_url" ] || die "$title has no file ending in $cli_suffix"
  fetch "$cli_url"
  mkdir -p "$bin_dir"
  tar -xzf "$tmp/${cli_url##*/}" -C "$tmp"
  install -m 755 "$tmp/iso-cli" "$bin_dir/iso-cli"
  say "Installed $bin_dir/iso-cli"
fi

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) if [ "$os" = Linux ] || [ "$cli" = 1 ]; then say "Add $bin_dir to PATH to run it from a shell."; fi ;;
esac
