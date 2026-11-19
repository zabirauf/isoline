# The Copilot sidecar (sidecar/copilot/live), which links the GitHub Copilot SDK. The Copilot
# CLI is not embedded; point COPILOT_CLI_PATH at one to use live AI.
{
  lib,
  buildGoModule,
  buildGo126Module ? null,
}:
let
  build = if buildGo126Module != null then buildGo126Module else buildGoModule;
in
build {
  pname = "copilot-sidecar";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../sidecar/copilot;
    fileset = lib.fileset.difference ../sidecar/copilot (lib.fileset.maybeMissing ../sidecar/copilot/bin);
  };
  modRoot = "live";
  vendorHash = "sha256-o67D8OovhOqmk+hulTpBH+HKjRDQNcb1bOJXOvuLkYg=";
  env.CGO_ENABLED = 0;
  ldflags = [
    "-s"
    "-w"
  ];
  doCheck = false;
  postInstall = ''
    mv $out/bin/live $out/bin/copilot-sidecar
  '';
  meta = {
    description = "JSON-RPC bridge between Isoline and the GitHub Copilot SDK";
    license = lib.licenses.mit;
    mainProgram = "copilot-sidecar";
  };
}
