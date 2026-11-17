{
  description = "Isoline: understand code without reading every line";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      packagesFor = pkgs: rec {
        copilot-sidecar = pkgs.callPackage ./nix/sidecar.nix { };
        iso-cli = pkgs.callPackage ./nix/iso-cli.nix { };
        isoline = pkgs.callPackage ./nix/isoline.nix { inherit copilot-sidecar; };
        default = isoline;
      };
    in
    {
      packages = forAllSystems packagesFor;

      apps = forAllSystems (pkgs: {
        iso-cli = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.iso-cli}/bin/iso-cli";
        };
      });

      overlays.default = final: _prev: {
        inherit (packagesFor final) isoline iso-cli;
      };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.isoline ];
          packages = [
            pkgs.rustfmt
            pkgs.clippy
          ];
        };
      });
    };
}
