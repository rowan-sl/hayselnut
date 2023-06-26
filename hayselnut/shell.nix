# A standalone shell definition that downloads and uses packages from `nixpkgs-esp-dev` automatically.
let
  nixpkgs-esp-dev = builtins.fetchGit {
    url = "https://github.com/mirrexagon/nixpkgs-esp-dev.git";

    # Optionally pin to a specific commit of `nixpkgs-esp-dev`.
    # rev = "<commit hash>";
  };

  pkgs = import <nixpkgs> { overlays = [ (import "${nixpkgs-esp-dev}/overlay.nix") ]; };
in
pkgs.mkShell {
  name = "esp-project";
  nativeBuildInputs = [ pkgs.pkgconfig ];
  buildInputs = with pkgs; [
    gcc-riscv32-esp32c3-elf-bin
    openocd-esp32-bin
    pkgconfig
    cmake
    ninja
    python3
    udev
  ];
  
  # fixes libstdc++ issues and libgl.so issues 
  LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib/:/run/opengl-driver/lib/";
  # fixes other stuff
  LIBCLANG_PATH = "${pkgs.llvmPackages_11.libclang.lib}/lib";
  TMPDIR="/tmp";
}
