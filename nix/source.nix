# The files the Rust, UI and sidecar builds read: everything except docs, eval data and
# screenshots, so editing those does not rebuild the packages.
{ lib }:
let
  fs = lib.fileset;
in
fs.toSource {
  root = ../.;
  fileset = fs.difference (fs.unions [
    ../Cargo.toml
    ../Cargo.lock
    ../.cargo
    ../crates
    ../app
    ../sidecar
  ]) (fs.unions [
    (fs.maybeMissing ../app/ui/screenshots)
    (fs.maybeMissing ../app/ui/node_modules)
    (fs.maybeMissing ../app/node_modules)
    (fs.maybeMissing ../app/ui/dist)
    (fs.maybeMissing ../sidecar/copilot/bin)
  ]);
}
