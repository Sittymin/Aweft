{
  description = "A Rust project using Slint on NixOS with Wayland";

  inputs = {
    # 指向你的 nixpkgs 源，unstable 通常包含最新的开发包
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # flake-utils 帮助我们简化为多系统编写配置的模板代码
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    # 使用 flake-utils 为所有默认系统 (x86_64-linux, aarch64-linux, etc.) 生成配置
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        # 为当前系统获取 pkgs
        pkgs = nixpkgs.legacyPackages.${system};

        # 把 Slint 在运行时需要动态加载的库放在一个列表里，方便管理
        runtimeLibs = with pkgs; [
          wayland
          libxkbcommon
          fontconfig
          # 添加 OpenGL 驱动实现
          libGL
        ];
      in
      {
        # 定义默认的开发环境
        devShells.default = pkgs.mkShell {
          # buildInputs 用于提供编译时依赖和命令行工具
          # 我们也把运行时库加进来，这是一个好习惯
          buildInputs =
            with pkgs;
            [
              rustc
              cargo
              pkg-config # 构建脚本可能需要
            ]
            ++ runtimeLibs;

          # --- 这是解决问题的关键部分 ---
          # 设置 LD_LIBRARY_PATH 环境变量
          # pkgs.lib.makeLibraryPath 会接收一个包列表，并生成一个正确的、
          # 以冒号分隔的路径字符串，指向这些包的 `lib` 目录。
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
        };
      }
    );
}
