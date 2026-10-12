{ pkgs ? import <nixpkgs> { } }:

let
  libraries = with pkgs; [
    fontconfig
    freetype
    libxkbcommon
    libxcb
    wayland
    vulkan-loader
    openssl
  ];
in
pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    pkg-config
    cmake
    clang
    awscli2
  ];

  buildInputs = libraries;
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

  shellHook = ''
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath libraries}:/run/opengl-driver/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  '';
}
