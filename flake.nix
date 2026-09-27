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
          # 插件 Guest 的编译目标：plugins/*/guest 下的 Rust 源码要编成
          # wasm 字节码提交为 plugins/*/guest.wasm，运行时由 wasmi 装载。
          targets = [
            "wasm32-unknown-unknown"
          ];
        };
        slintRuntimeDeps = with pkgs; [
          wayland
          libxkbcommon
          fontconfig # Slint 和 libass 都需要
          vulkan-loader
          mesa
          # GStreamer 视频硬解运行时（Intel：VA-API，iHD 驱动）
          libva
          intel-media-driver
          # libgstwayland-1.0（waylandsink 子 surface 嵌入）
          gst_all_1.gst-plugins-bad
        ];
        libassRuntimeDeps = with pkgs; [
          freetype
          fribidi
          harfbuzz
        ];

        buildToolsAndDevLibs = with pkgs; [
          pkg-config
          curl
          gnutar
          gzip
          gnupatch

          # libass build
          autoconf
          automake116x
          libtool
          nasm

          # bindgen + clangd fallback compiler
          llvmPackages.clang
          llvmPackages.clang-tools

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
          nativeBuildInputs = buildToolsAndDevLibs ++ [
            rustToolchain
            pkgs.rustPlatform.bindgenHook
          ];

          # buildInputs 包含了所有最终程序需要链接的“零件”
          buildInputs = libassRuntimeDeps ++ slintRuntimeDeps ++ [ pkgs.glibc.dev ];

          # 环境变量可以暂时保留，以防万一
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (libassRuntimeDeps ++ slintRuntimeDeps);
          # 让 clang 找到库
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          # vendored libass headers for clangd — generated at runtime to avoid
          # Nix pure-eval filtering of git submodules (meson fs.copyfile pattern).
          shellHook = ''
            vendored_inc="$TMPDIR/libass-vendored-$$/include"
            if [ -d "$PWD/aweft-libass/third_party/libass/libass" ]; then
              mkdir -p "$vendored_inc/ass"
              ln -sf "$PWD/aweft-libass/third_party/libass/libass/ass.h" "$vendored_inc/ass/ass.h"
              ln -sf "$PWD/aweft-libass/third_party/libass/libass/ass_types.h" "$vendored_inc/ass/ass_types.h"
              export C_INCLUDE_PATH="$vendored_inc''${C_INCLUDE_PATH:+:$C_INCLUDE_PATH}"
              export CPLUS_INCLUDE_PATH="$vendored_inc''${CPLUS_INCLUDE_PATH:+:$CPLUS_INCLUDE_PATH}"
            fi
            # Provide explicit compile flags for clangd fallback 
            # ( ensures vendored headers are found even when compile_commands.json is absent).
            # This file is gitignored and regenerated on each shell entry.
            if [ -n "''${NIX_CFLAGS_COMPILE:-}" ]; then
              mkdir -p "$PWD/aweft-libass"
              {
                echo "-I$vendored_inc"
                for flag in $NIX_CFLAGS_COMPILE; do
                  printf "%s\n" "$flag"
                done
              } > "$PWD/aweft-libass/compile_flags.txt"
            fi
          '';

          SLINT_BACKEND = "winit-femtovg-wgpu";
          # 实时更新
          SLINT_LIVE_PREVIEW = 1;
          # live preview 把整个宿主 .slint 挪到运行期当场编译（rust.rs:172 整份文档分岔，
          # 嵌的是构建机绝对路径），那一刻编译器读「进程环境」选内建类型表
          # （compiler/lib.rs:276）；ComponentContainer/component-factory 是实验类型，
          # 没有它建窗即报 Unknown type 'component-factory'。build.rs 的 set_var/rustc-env
          # 只够构建期用，到不了运行中的二进制，运行期这份只能由 shell 出。
          SLINT_ENABLE_EXPERIMENTAL_FEATURES = 1;
          # 显示代码的所有 log 输出
          RUST_LOG = "warn,aweft_gui=trace,aweft_model=trace,aweft_ass=trace,aweft_collab=trace,aweft_convert=trace,aweft_libass=trace";
        };
      }
    );
}
