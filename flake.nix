{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      crane,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rust-toolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rust-toolchain;

        mpd-herald = craneLib.buildPackage {
          src = craneLib.cleanCargoSource ./.;

          buildInputs = [ ];
        };
      in
      {
        packages.default = mpd-herald;

        devShells.default = craneLib.devShell {
          inputsFrom = [ mpd-herald ];

          packages = with pkgs; [
            cargo-edit
          ];
        };
      }
    );
}
