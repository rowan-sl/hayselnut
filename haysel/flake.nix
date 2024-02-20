{
  description = "Server for the Hayselnut project";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    let
      overlays = [ (import rust-overlay ) ];
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system overlays;
      };
      rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      # tell crane to use this toolchain
      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
      # cf. https://crane.dev/API.html#libcleancargosource
      src = craneLib.cleanCargoSource ./.;
      # compile-time + shell
      nativeBuildInputs = with pkgs; [ rustToolchain pkg-config clang mold ];
      # runtime (nix build)
      buildInputs = with pkgs; [ ]; # needed system libraries
      cargoArtifacts = craneLib.buildDepsOnly { inherit src buildInputs nativeBuildInputs; };
      haysel-bin = craneLib.buildPackage ({ inherit src buildInputs nativeBuildInputs cargoArtifacts; });
    in
    {
      packages.${system} = {
        # so bin can be spacifically built, or just by default
        inherit haysel-bin;
        default = haysel-bin;
      };
      devShells.${system}.default = pkgs.mkShell {
        inherit buildInputs;
        nativeBuildInputs = [ pkgs.rust-analyzer-unwrapped ] ++ nativeBuildInputs;
        RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        shellHook = ''
        if [ -n "$\{EXEC_THIS_SHELL}" ]; then 
          exec $EXEC_THIS_SHELL
        fi
        '';
      };
    };
}
