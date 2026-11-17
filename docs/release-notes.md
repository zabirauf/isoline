Isoline indexes a repository with tree-sitter and shows it as function cards, a dependency
map and call flow, with the code beside them in Monaco.

## Downloads

| System | App | Command line |
| --- | --- | --- |
| macOS, Apple silicon | `Isoline_*_aarch64.dmg` | `iso-cli_*_macos-arm64.tar.gz` |
| macOS, Intel | `Isoline_*_x64.dmg` | `iso-cli_*_macos-x64.tar.gz` |
| Windows, x64 | `Isoline_*_x64-setup.exe` or `.msi` | `iso-cli_*_windows-x64.zip` |
| Windows, arm64 | `Isoline_*_arm64-setup.exe` | `iso-cli_*_windows-arm64.zip` |
| Linux, x64 | `.AppImage`, `.deb` or `.rpm` (`amd64` / `x86_64`) | `iso-cli_*_linux-x64.tar.gz` |
| Linux, arm64 | `.AppImage`, `.deb` or `.rpm` (`arm64` / `aarch64`) | `iso-cli_*_linux-arm64.tar.gz` |

`SHA256SUMS.txt` lists a checksum for every file.

**Nix:** `nix run github:zabirauf/isoline` builds the app from source; `nix run
github:zabirauf/isoline#iso-cli -- --help` gives the command line. The flake also exports
`overlays.default`.

## Unsigned builds

These builds are not code-signed yet.

- **macOS:** the first time, right-click Isoline and choose Open, or run
  `xattr -dr com.apple.quarantine /Applications/Isoline.app`.
- **Windows:** SmartScreen warns about an unknown publisher; choose More info, then Run anyway.

## AI

Summaries, module names and colors come from GitHub Copilot and are off until you turn them
on per repo. Everything else works offline.
