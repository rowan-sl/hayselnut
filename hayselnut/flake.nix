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
      ldproxy = pkgs.callPackage ./nix-ldproxy { };
      esp-idf = pkgs.callPackage ./nix-esp-idf { };
      esp-idf-riscv = esp-idf.override {
        toolsToInclude = [
          "riscv32-esp-elf"
          "openocd-esp32"
          "riscv32-esp-elf-gdb"
        ];
      };
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
      # compile-time
      nativeBuildInputs = with pkgs; [ rustToolchain pkg-config ldproxy ];
      # runtime
      buildInputs = with pkgs; [ esp-idf-riscv ]; # needed system libraries
      cargoArtifacts = craneLib.buildDepsOnly { inherit src buildInputs nativeBuildInputs; };
      bin = craneLib.buildPackage ({ inherit src buildInputs nativeBuildInputs cargoArtifacts; });
    in
    {
      # removed becuase a builder for this doesn't really make sense
      # packages.${system} = {
      #   # so bin can be spacifically built, or just by default
      #   inherit bin;
      #   default = bin;
      # };
      devShells.${system}.default = pkgs.mkShell {
        inherit buildInputs;
        nativeBuildInputs = [
          pkgs.cargo-espflash          
          pkgs.rust-analyzer-unwrapped
        ] ++ nativeBuildInputs;
        # fixes libstdc++ issues 
        LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib/";
        # fixes other stuff
        LIBCLANG_PATH = "${pkgs.llvmPackages_11.libclang.lib}/lib";
        TMPDIR="/tmp";
        RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
      };
    };
}
