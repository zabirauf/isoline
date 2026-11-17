#!/bin/sh
# Builds the Copilot sidecar as bin/copilot-sidecar-<rust target triple>[.exe], the name
# Tauri's externalBin expects.
#
#   ./build.sh [--mock-only] [--embed-cli] [target-triple]
#
# The target defaults to $TAURI_ENV_TARGET_TRIPLE (set by `tauri build`), then to the host
# triple from `rustc -vV`.
#
# By default the live build (./live, which links the GitHub Copilot SDK) is tried first. The
# SDK needs Go 1.26; GOTOOLCHAIN=auto lets an older go fetch that toolchain once. If the
# live build fails, the mock-only build (this module) is produced instead, with a warning:
# it speaks the full protocol but answers model calls with a "not available" error unless
# run with --mock.
#
# --mock-only   skip the live build.
# --embed-cli   run the SDK's bundler first, which downloads the Copilot CLI release for the
#               target and embeds it in the binary. Without it the live sidecar needs
#               COPILOT_CLI_PATH at run time.
#
# Environment: GO (go binary, default `go`), OUT_DIR (default ./bin).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
GO=${GO:-go}
if ! command -v "$GO" >/dev/null 2>&1; then
  if [ -x /opt/homebrew/bin/go ]; then GO=/opt/homebrew/bin/go; else echo "build.sh: go not found" >&2; exit 1; fi
fi

mock_only=0
embed=0
triple=""
for arg in "$@"; do
  case "$arg" in
    --mock-only) mock_only=1 ;;
    --embed-cli) embed=1 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    -*) echo "build.sh: unknown option $arg" >&2; exit 2 ;;
    *) triple=$arg ;;
  esac
done

# Inside `tauri build --target <triple>`, Tauri names the target it is building for.
[ -z "$triple" ] && triple=${TAURI_ENV_TARGET_TRIPLE:-}
if [ -z "$triple" ]; then
  if ! command -v rustc >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
  fi
  triple=$(rustc -vV | sed -n 's/^host: //p')
fi

case "$triple" in
  aarch64-apple-darwin) goos=darwin; goarch=arm64 ;;
  x86_64-apple-darwin) goos=darwin; goarch=amd64 ;;
  x86_64-unknown-linux-gnu|x86_64-unknown-linux-musl) goos=linux; goarch=amd64 ;;
  aarch64-unknown-linux-gnu|aarch64-unknown-linux-musl) goos=linux; goarch=arm64 ;;
  x86_64-pc-windows-msvc|x86_64-pc-windows-gnu) goos=windows; goarch=amd64 ;;
  aarch64-pc-windows-msvc) goos=windows; goarch=arm64 ;;
  *) echo "build.sh: unsupported target $triple" >&2; exit 2 ;;
esac

ext=""
[ "$goos" = windows ] && ext=".exe"
out_dir=${OUT_DIR:-$here/bin}
mkdir -p "$out_dir"
out="$out_dir/copilot-sidecar-$triple$ext"

export GOOS=$goos GOARCH=$goarch CGO_ENABLED=0

if [ "$mock_only" = 0 ]; then
  if (
    cd "$here/live"
    if [ "$embed" = 1 ]; then
      GOOS= GOARCH= GOTOOLCHAIN=auto "$GO" run github.com/github/copilot-sdk/go/cmd/bundler -platform "$goos/$goarch"
    fi
    GOTOOLCHAIN=auto "$GO" build -trimpath -o "$out" .
  ); then
    echo "built $out (live)"
    exit 0
  fi
  echo "build.sh: the live build failed; building the mock-only sidecar instead" >&2
fi

(cd "$here" && "$GO" build -trimpath -o "$out" .)
echo "built $out (mock only)"
