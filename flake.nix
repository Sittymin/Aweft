{
  description = "A Rust project using Slint on NixOS with Wayland";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/0e251e24a4f24e036a084b6b4b2d2491af4167f4";
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
          automake116x
          libtool
          nasm

          # bindgen
          libclang.lib

          # gStreamer dependencies
          glib
          gst_all_1.gstreamer
          gst_all_1.gst-plugins-base
          gst_all_1.gst-plugins-good
          gst_all_1.gst-plugins-bad
          gst_all_1.gst-plugins-ugly
          gst_all_1.gst-libav
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
          # 让 bindgen 调用的 libclang 找到 C 标准库头文件 (Nix 下 /usr/include 不存在)
          BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.glibc.dev}/include";

          SLINT_BACKEND = "winit-femtovg-wgpu";
          # 实时更新
          SLINT_LIVE_PREVIEW = 1;
          # 显示代码的所有 log 输出
          RUST_LOG = "warn,aweft_gui=trace,aweft_model=trace,aweft_ass=trace,aweft_collab=trace,aweft_convert=trace,aweft_libass=trace";
        };
      }
    );
}
