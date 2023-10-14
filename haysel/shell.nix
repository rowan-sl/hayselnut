{ pkgs ? import <nixpkgs> { overlays = [ (import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz)) ]; } }:

with pkgs;

mkShell {
  name = "rust";
  nativeBuildInputs = [
    pkg-config
  ];
  buildInputs = [
    clang
    lld
    mold
    # needed for cargo-espflash
    systemd
  ];
  # fixes libstdc++ issues and libgl.so issues
  LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib/:/run/opengl-driver/lib/";
  # fixes other stuff
  LIBCLANG_PATH = "${pkgs.llvmPackages_11.libclang.lib}/lib";
  TMPDIR="/tmp";
}
