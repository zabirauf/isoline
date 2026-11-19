# The official prebuilt libghostty-vt release that `ghostty-vt-sys` links on macOS. Nix builds
# have no network, so it is fetched here (sha256 from the crate's GHOSTTY.lock) and passed as
# GHOSTTY_RELEASE_DIR. Linux does not use it: there the terminal uses the WebAssembly build.
{ fetchurl, linkFarm }:
let
  version = "1.0.1";
in
linkFarm "libghostty-vt-${version}-release" [
  {
    name = "ghostty-vt-${version}.xcframework.zip";
    path = fetchurl {
      url = "https://release.files.ghostty.org/libghostty-vt/${version}/ghostty-vt-${version}.xcframework.zip";
      sha256 = "c0f9631277f726154c8f521954d17cdc41aac4626fc0df8cf925a19ee58e829b";
    };
  }
]
