{ pkgs ? import <nixpkgs> { } }:

let
  runtimeLibs = with pkgs; [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
    pipewire
    pam
    dbus
  ];
in
pkgs.mkShell {
  name = "acanthus";

  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    rust-analyzer
    cargo-nextest
    cargo-deny
    pkg-config
    clang
    mold
    llvmPackages.libclang
  ];

  buildInputs = runtimeLibs ++ (with pkgs; [
    wayland-protocols
    wayland-scanner
    fontconfig
    freetype
    vulkan-headers
    vulkan-validation-layers
    vulkan-tools
    shaderc
  ]);

  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
  VK_LAYER_PATH = "${pkgs.vulkan-validation-layers}/share/vulkan/explicit_layer.d";
  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
  RUST_BACKTRACE = "1";
}
