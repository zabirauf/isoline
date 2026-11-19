# iso-cli: index a repo and query it from the terminal (index, file, flow, impact, bench, ...).
{
  lib,
  stdenv,
  rustPlatform,
  pkg-config,
  sqlite,
  ghostty-vt-release,
}:
rustPlatform.buildRustPackage {
  pname = "iso-cli";
  version = "0.1.0";
  src = import ./source.nix { inherit lib; };
  cargoLock.lockFile = ../Cargo.lock;
  # rusqlite bundles SQLite, but the link line still names libsqlite3 under Nix's cc wrapper.
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ sqlite ];
  # The terminal's native parser (libghostty-vt), prebuilt, on macOS.
  env = lib.optionalAttrs stdenv.hostPlatform.isDarwin {
    GHOSTTY_RELEASE_DIR = "${ghostty-vt-release}";
  };
  cargoBuildFlags = [
    "-p"
    "iso-cli"
  ];
  # The test suite runs in CI; several tests need git and a writable home.
  doCheck = false;
  meta = {
    description = "Command-line interface to the Isoline code index";
    homepage = "https://github.com/zabirauf/isoline";
    license = lib.licenses.mit;
    mainProgram = "iso-cli";
  };
}
