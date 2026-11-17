# iso-cli: index a repo and query it from the terminal (index, file, flow, impact, bench, ...).
{ lib, rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "iso-cli";
  version = "0.1.0";
  src = import ./source.nix { inherit lib; };
  cargoLock.lockFile = ../Cargo.lock;
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
