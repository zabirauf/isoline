# The Isoline desktop app (Tauri 2), built from source with the Copilot sidecar beside it.
{
  lib,
  stdenv,
  rustPlatform,
  cargo-tauri,
  nodejs,
  importNpmLock,
  pkg-config,
  wrapGAppsHook3,
  webkitgtk_4_1,
  gtk3,
  glib-networking,
  libsoup_3,
  librsvg,
  openssl,
  sqlite,
  copilot-sidecar,
  ghostty-vt-release,
}:
rustPlatform.buildRustPackage {
  pname = "isoline";
  version = "0.1.0";
  src = import ./source.nix { inherit lib; };

  cargoLock.lockFile = ../Cargo.lock;

  # The UI's npm dependencies, straight from its lockfile.
  npmDeps = importNpmLock { npmRoot = ../app/ui; };
  npmRoot = "app/ui";

  nativeBuildInputs = [
    cargo-tauri.hook
    nodejs
    importNpmLock.npmConfigHook
    pkg-config
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [ wrapGAppsHook3 ];

  buildInputs = [ sqlite ] ++ lib.optionals stdenv.hostPlatform.isLinux [
    webkitgtk_4_1
    gtk3
    glib-networking
    libsoup_3
    librsvg
    openssl
  ];

  # The terminal's native parser (libghostty-vt), prebuilt, on macOS.
  env = lib.optionalAttrs stdenv.hostPlatform.isDarwin {
    GHOSTTY_RELEASE_DIR = "${ghostty-vt-release}";
  };

  # `tauri build` runs in app/src-tauri; the workspace target dir stays at the root.
  buildAndTestSubdir = "app/src-tauri";

  # The UI is built in preBuild and the sidecar comes from its own derivation, so Tauri's
  # beforeBuildCommand is dropped (a JSON merge patch with null; flags must not contain spaces).
  tauriBuildFlags = [
    "--config"
    (builtins.toJSON { build.beforeBuildCommand = null; })
  ];

  preBuild = ''
    (cd app/ui && npm run build)
    install -Dm755 ${lib.getExe copilot-sidecar} \
      sidecar/copilot/bin/copilot-sidecar-${stdenv.hostPlatform.rust.rustcTarget}
  '';

  doCheck = false;

  meta = {
    description = "Desktop code map: function cards, dependency map and call flow for any repo";
    homepage = "https://github.com/zabirauf/isoline";
    license = lib.licenses.mit;
    mainProgram = "isoline";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
