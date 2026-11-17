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

  # `tauri build` runs in app/src-tauri; the workspace target dir stays at the root.
  buildAndTestSubdir = "app/src-tauri";

  # The sidecar comes from its own derivation, so the build only compiles the UI.
  tauriBuildFlags = [
    "--config"
    (builtins.toJSON { build.beforeBuildCommand = "npm --prefix ui run build"; })
  ];

  preBuild = ''
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
