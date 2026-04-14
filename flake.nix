{
  description = "Aegis-Boot - Reproducible build environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            (rustChannelOf {
              rustToolchain = rustPlatform.buildRustEnv {
                rustc = rustVersions.stable;
                Cargo = rustVersions.stable;
              };
            }.rust.override {
              extensions = [ "rustfmt" "clippy" ];
            })
            gcc
            pkg-config
            openssl
            python311
            python311Packages.pip
            nasm
            uuid
            iasl
            git
          ];

          RUST_VERSION = "1.75.0";

          shellHook = ''
            echo "Aegis-Boot Build Environment"
            echo "================================"
            echo "Rust: $RUST_VERSION"
            echo "Nixpkgs: ${nixpkgs.lib.version}"
          '';
        };
      }
    );
}
