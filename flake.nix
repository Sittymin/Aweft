{
  description = "A Rust project using Slint on NixOS with Wayland";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
          ];
        };
        slintRuntimeDeps = with pkgs; [
          wayland
          libxkbcommon
          fontconfig # Slint 和 libass 都需要
          vulkan-loader
          mesa
        ];
        libassRuntimeDeps = with pkgs; [
          freetype
          fribidi
          harfbuzz
        ];

        buildToolsAndDevLibs = with pkgs; [
          pkg-config

          # libass build
          autoconf
          automake
          libtool
          nasm

          # bindgen
          libclang.lib

          # gStreamer dependencies
          glib
          # gstreamer
          gst_all_1.gstreamer
          gst_all_1.gst-plugins-base
          gst_all_1.gst-plugins-good
          gst_all_1.gst-plugins-bad
          gst_all_1.gst-plugins-ugly
          gst_all_1.gst-libav
          gst_all_1.gst-vaapi
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = buildToolsAndDevLibs ++ [ rustToolchain ];

          # buildInputs 包含了所有最终程序需要链接的“零件”
          buildInputs = libassRuntimeDeps ++ slintRuntimeDeps;

          # 环境变量可以暂时保留，以防万一
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (libassRuntimeDeps ++ slintRuntimeDeps);
          # 让 clang 找到库
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

          SLINT_BACKEND = "winit-femtovg-wgpu";
        };
      }
    );
}
