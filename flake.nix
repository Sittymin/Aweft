{
  description = "A Rust project using Slint on NixOS with Wayland";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
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
          rustc
          cargo
          pkg-config

          # libass build
          autoconf
          automake
          libtool
          nasm

          # bindgen
          libclang.lib
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = buildToolsAndDevLibs;

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
