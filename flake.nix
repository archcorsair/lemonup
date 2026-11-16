{
  description = "Terminal addon manager for World of Warcraft (Retail and WoW Forever)";

  # The lemonup Cachix cache goes in a nixConfig block here
  # (extra-substituters and extra-trusted-public-keys) once the cache exists.

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Nixpkgs 26.11 dropped x86_64-darwin. The 26.05 branch still builds for
    # Intel Macs and gets security fixes until the end of 2026.
    nixpkgs-x86_64-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-x86_64-darwin,
    }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      pkgsFor =
        system:
        (if system == "x86_64-darwin" then nixpkgs-x86_64-darwin else nixpkgs).legacyPackages.${system};
      forAllSystems = f: lib.genAttrs systems (system: f system (pkgsFor system));
    in
    {
      overlays.default = final: _prev: {
        lemonup = final.callPackage ./nix/package.nix { };
      };

      packages = forAllSystems (
        _system: pkgs: rec {
          lemonup = pkgs.callPackage ./nix/package.nix { };
          default = lemonup;
        }
      );

      apps = forAllSystems (
        system: _pkgs: {
          default = {
            type = "app";
            program = lib.getExe self.packages.${system}.lemonup;
            meta.description = "Run LemonUp";
          };
        }
      );

      devShells = forAllSystems (
        _system: pkgs: {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
            ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        }
      );

      formatter = forAllSystems (_system: pkgs: pkgs.nixfmt-tree);

      checks = forAllSystems (
        system: _pkgs: {
          lemonup = self.packages.${system}.lemonup;
        }
      );
    };
}
