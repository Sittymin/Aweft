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
          # Android SDK 组件（composeAndroidPackages 拉取）要先接受 Google 许可，
          # 否则求值直接报错。接受动作只影响本工程的 nix 闭包。
          config = {
            android_sdk.accept_license = true;
            # SDK/NDK 在 nixpkgs 里标了 unfree；里层是 sdk-tools-linux-*.zip、
            # platform-tools_*.zip 这类下载件，逐个点名放行太脆。
            # 这里开 unfree 只影响本工程 devShell 的 pkgs 实例，不碰系统配置。
            allowUnfree = true;
          };
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
          ];
          # 插件 Guest 的编译目标：plugins/*/guest 下的 Rust 源码要编成
          # wasm 字节码提交为 plugins/*/guest.wasm，运行时由 wasmi 装载。
          # Android 目标：aarch64 真机 + x86_64 模拟器，见下方的 androidSdk 与
          # shellHook 里的链接器/NDK 环境变量。
          targets = [
            "wasm32-unknown-unknown"
            "aarch64-linux-android"
            "x86_64-linux-android"
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

        # Android 工具链（SDK + NDK 全由 nix 提供，不依赖本机 ~/Android/Sdk）。
        # 版本选择：platform 35 对应 targetSdk；build-tools 与之同期；
        # NDK 取 r29 最终版（r30 在部分 nixpkgs 快照里只有 rc，r29 各件齐全；
        # Rust、Slint（minSdk 26）、GStreamer 通用包（要求 NDK r25+）都满足）。
        # emulator 不收进闭包（体积大，沿用本机已有的；adb 用下面的 platform-tools）。
        ndkVersion = "29.0.14206865";
        androidSdk = pkgs.androidenv.composeAndroidPackages {
          platformToolsVersion = "37.0.1";
          # 34 只当一个用途：xbuild 读不了 API 35 的 android.jar（见下面 xbuildSdk）。
          platformVersions = [ "34" "35" ];
          buildToolsVersions = [ "35.0.0" ];
          includeNDK = true;
          ndkVersions = [ ndkVersion ];
        };
        # compose 返回的是包集合（androidsdk、ndk-bundle、platform-tools……），
        # 真正的 SDK 根是其中的 androidsdk 合成件（各件链接进 libexec/android-sdk）。
        androidSdkRoot = "${androidSdk.androidsdk}/libexec/android-sdk";

        # xbuild（可执行文件名 `x`）：出 APK 的那条链用它，本体不在 nixpkgs 里
        #（by-name 里那个 cargo-xbuild 是编 core/std 的另一回事）。
        # 只能 pin git：crates.io 那份 0.2.0（最后一次发版，2022-12）少两样我们正在用的
        # 东西——`android.assets` 与 `generic.runtime_libs` 这两个配置项它压根没有，
        # 而片 B 的字幕字体/面板进包就落在这两处（`xbuild/src/config.rs:410,440`）；
        # 且它 `(Platform::Android, Opt::Release)` 无条件默认 `.aab`（发布后的 c8efbd8
        # 才收窄成「只有走 gradle 才默认 aab」），同一条命令在装机侧就拿不到 .apk。
        # rev 取 e67b501 = `sources/xbuild` 那份阅读检出：仓库里引的 xbuild 行号
        #（`cargo/mod.rs:295-298`、`lib.rs:622`）与已装机的那份产物都按这一份核过。
        xbuildRev = "e67b501cbe0e9ca5436c223aa7ec0fe5e27544d1";
        xbuildUpstream = pkgs.fetchFromGitHub {
          owner = "rust-mobile";
          repo = "xbuild";
          rev = xbuildRev;
          hash = "sha256-k32rD3hlVXfUKo6/QrKkvoBDHKkZ37jKuXSWn+kTd08=";
        };
        # 上游把 Cargo.lock 放进了 .gitignore，git pin 因此不带锁，依赖图无从取；
        # 光给 `cargoLock.lockFile` 也不够——cargoSetupHook 的 postPatch 会拿
        # 源码树里的 Cargo.lock 与 vendor 目录比对，缺了直接报
        # "Missing Cargo.lock from src"。所以把锁并进源码树。
        # 这份锁是本 rev 跑 `cargo generate-lockfile` 生成的，存在 `nix/xbuild.Cargo.lock`
        #（479 个包，锁里没有 git 源）；换 xbuildRev 必须重新生成，否则对不上清单。
        xbuildSrc = pkgs.runCommand "aweft-xbuild-src" { } ''
          cp -R --no-preserve=mode ${xbuildUpstream} $out
          chmod -R u+w $out
          cp ${./nix/xbuild.Cargo.lock} $out/Cargo.lock
        '';
        xbuild = pkgs.rustPlatform.buildRustPackage {
          pname = "xbuild";
          version = "0.2.0+e67b501";
          src = xbuildSrc;
          cargoLock.lockFile = ./nix/xbuild.Cargo.lock;
          # 仓库根 workspace 的 default-members = ["xbuild"]，cargo 默认只编这一个成员。
          # 依赖图里有 openssl-sys（native-tls 那条路），它的 build.rs 要 pkg-config
          # 找到 OpenSSL 才肯继续；不给就 panic，不是警告。
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ];
          # xbuild 的测试要真机，且它联网下载 SDK。
          doCheck = false;
        };

        # NDK 的版本化 clang 所在目录。版本单点取自上面的 ndkVersion；
        # shellHook 里那份是同一目录的运行期 glob。
        ndkBin = "${androidSdkRoot}/ndk/${ndkVersion}/toolchains/llvm/prebuilt/linux-x86_64/bin";
        # 裸名 clang/clang++：xbuild 的 use_android_ndk 把 cc/cxx/linker 一律写成裸名
        # clang（xbuild/src/cargo/mod.rs:295-298），盖掉 shellHook 给的绝对路径，
        # 而 buildInputs 里 llvmPackages.clang 那个裸名是宿主 clang，链 Android 缺库。
        # 必须是「绝对路径 exec」的脚本，不能是软链：NDK 那份 aarch64-*-clang 自己是个
        # bash 壳，靠 `dirname "$0"` 找真 clang，$0 落在软链所在目录就会无限自我递归
        #（实测 15 秒 fork 出两千多个 bash+clang、吃掉十来 GB，把机器推进 OOM）。
        ndkClangShim = pkgs.writeShellScriptBin "clang" ''
          exec ${ndkBin}/aarch64-linux-android26-clang "$@"
        '';
        ndkClangxxShim = pkgs.writeShellScriptBin "clang++" ''
          exec ${ndkBin}/aarch64-linux-android26-clang++ "$@"
        '';
        # 只在这条命令里生效。直接把上面两个塞进 nativeBuildInputs 会连桌面构建的
        # clang 一起换掉（bindgen、clangd、aweft-libass 都要宿主那份）。
        # 包装件占用的名字就是 `x`：本机 ~/.cargo/bin 那份 xbuild 因此在壳里不可达，
        # 也走不出「裸 clang 是宿主那份、链 Android 缺库」这条错路。
        # 里面用绝对路径调 flake 建的 xbuild，且 shim 排在 PATH 最前。
        xbuildWithNdkClang = pkgs.writeShellScriptBin "x" ''
          PATH="${ndkClangShim}/bin:${ndkClangxxShim}/bin:$PATH" exec "${xbuild}/bin/x" "$@"
        '';
        # xbuild 只读 ~/.cache/x/Android.sdk 下 platforms/android-<targetSdk>/android.jar
        # 这一份文件（xbuild/src/lib.rs:622），而它的 apk crate 解析不了 API 35 资源表的
        # FlagOffset16 编码（成因见 docs/tmp/android-port.md「已验证」第 23 条），
        # 所以给它一棵镜像树：除了这份 jar 换成 34 的那份，其余原样链回去。
        xbuildSdk = pkgs.runCommand "aweft-xbuild-android-sdk" { } ''
          mkdir -p $out/platforms/android-35
          for e in ${androidSdkRoot}/*; do
            n=$(basename "$e")
            [ "$n" = platforms ] || ln -s "$e" "$out/$n"
          done
          for e in ${androidSdkRoot}/platforms/android-35/*; do
            n=$(basename "$e")
            [ "$n" = android.jar ] || ln -s "$e" "$out/platforms/android-35/$n"
          done
          ln -s ${androidSdkRoot}/platforms/android-34/android.jar \
            $out/platforms/android-35/android.jar
        '';

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
          # libass 的 Android 依赖（freetype/fribidi/harfbuzz）用 meson 交叉编，
          # 见 aweft-libass/android-deps.sh；fontconfig 在 Android 上禁用。
          meson
          ninja

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
        # 单独可建：换版本时只探这一份的 cargoHash，不用先把整个 devShell 拉起来。
        packages.xbuild = xbuild;

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = buildToolsAndDevLibs ++ [
            rustToolchain
            pkgs.rustPlatform.bindgenHook
            xbuildWithNdkClang
          ];

          # buildInputs 包含了所有最终程序需要链接的“零件”
          buildInputs = libassRuntimeDeps ++ slintRuntimeDeps ++ [ pkgs.glibc.dev ] ++ [
            androidSdk.androidsdk
            # xbuild 打包 APK 要 JDK；headless 足够（不需要 GUI 签名工具）。
            pkgs.jdk17_headless
          ];

          # 环境变量可以暂时保留，以防万一
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (libassRuntimeDeps ++ slintRuntimeDeps);
          # 让 clang 找到库
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          # vendored libass headers for clangd — generated at runtime to avoid
          # Nix pure-eval filtering of git submodules (meson fs.copyfile pattern).
          shellHook = ''
            # 这台机器整棵 / 是 tmpfs，进壳时 nix 给的 TMPDIR（/tmp/nix-shell.XXXX）
            # 就在内存里：clang 的临时 .s 与打包中间件全在吃 RAM。挪到磁盘上。
            export TMPDIR="''${XDG_CACHE_HOME:-$HOME/.cache}/aweft/tmp"
            mkdir -p "$TMPDIR"
            # NDK 在 SDK 里按版本分子目录（ndk/<版本号>/），glob 取之，
            # 不把版本号硬编码两遍（与上面的 ndkVersions 保持单点）。
            # 放 shellHook 而不用上面的纯环境变量：glob 与变量引用要按序求值。
            ndk_glob="$(echo ${androidSdkRoot}/ndk/*/)"
            export ANDROID_NDK_ROOT="''${ndk_glob%/}"
            ndk_bin="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin"
            # Rust 链接器：NDK 自带的版本化 clang 包装（已带 --target 与 sysroot）。
            # minSdk 取 26（Slint 硬性要求），包装名里的 26 即 minSdk，不是 NDK 版本。
            export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk_bin/aarch64-linux-android26-clang"
            export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$ndk_bin/x86_64-linux-android26-clang"
            # 含 C 编译的 build.rs（如 aweft-libass 的 cc/autotools）走同一套包装。
            # cc 包按 CC_<triple 横线> → CC_<triple 下划线> → TARGET_CC → CC 取第一个
            # 命中的（cc-1.4.0/src/lib.rs:4213），autotools 包再用 cc 包选出的编译器；
            # 横线式那份 xbuild 会自己占掉，故 aweft-libass/build.rs 里显式删掉它。
            export CC_aarch64_linux_android="$ndk_bin/aarch64-linux-android26-clang"
            export CXX_aarch64_linux_android="$ndk_bin/aarch64-linux-android26-clang++"
            export AR_aarch64_linux_android="$ndk_bin/llvm-ar"
            export CC_x86_64_linux_android="$ndk_bin/x86_64-linux-android26-clang"
            # 目标平台的 pkg-config 搜索路径。pkg-config 这个 Rust crate 在交叉构建
            # 时只认 TARGET_PKG_CONFIG_PATH（build.rs 里 TARGET 由 cargo 注入），
            # 且它一旦存在就整体替换 PKG_CONFIG_PATH，所以主机 .pc 目录必须拼在最后：
            # check 阶段的版本探测靠它们通过（只探测不链接）。
            # 拼出来的顺序就是选中优先级，同名 .pc 谁在前谁中选：
            #   1. libass 的 Android 预编件（freetype2/fribidi/harfbuzz）；
            #   2. GStreamer Android 通用包 arm64（gstreamer-*/glib/gobject/gio）——
            #      该目录里也带一份同名 freetype2/fribidi/harfbuzz，故必须排在 1 之后，
            #      否则 libass 会链到通用包那份；
            #   3. 主机 .pc 兜底。
            # 只在目录存在时导出，没跑过 android-deps.sh、没下通用包的机器不受影响。
            target_pkgs=""
            if [ -d "$PWD/aweft-libass/third_party/android-deps" ]; then
              target_pkgs="$PWD/aweft-libass/third_party/android-deps/share/pkgconfig:$PWD/aweft-libass/third_party/android-deps/lib/pkgconfig"
            fi
            gst_android_pkgs="''${GST_ANDROID_PKGCONFIG:-/home/Sittymin/Development/sources/gstreamer-android/arm64/lib/pkgconfig}"
            if [ -d "$gst_android_pkgs" ]; then
              target_pkgs="''${target_pkgs:+$target_pkgs:}$gst_android_pkgs"
              # 导出的同一份路径还喂给 aweft-android/build.rs：通用包全是 .a，插件
              # 不会被运行时扫描，链接行由它按 <pkgconfig>/../gstreamer-1.0 逐个给出。
              export GST_ANDROID_PKGCONFIG="$gst_android_pkgs"
            fi
            if [ -n "$target_pkgs" ]; then
              export TARGET_PKG_CONFIG_PATH="$target_pkgs:''${PKG_CONFIG_PATH:-}"
            fi
            # xbuild（可执行文件名 `x`）只认死路径 ~/.cache/x/{Android.sdk,Android.ndk}，
            # 不看 ANDROID_HOME/ANDROID_NDK_ROOT（xbuild/src/lib.rs 的 android_jar()/
            # android_ndk()），所以把 nix 闭包里这两份软链过去，省下一次 NDK 下载。
            # Android.sdk 给的是上面那棵 xbuildSdk 镜像（android-35 的 jar 换成 34 那份），
            # 不是 $ANDROID_SDK_ROOT 本身。
            # Android.ndk 必须是 NDK 的 *sysroot* 而不是 NDK 根：use_android_ndk
            # 查的是 <Android.ndk>/usr/lib/<三元组>/<api>，并把 --sysroot 直接指给它，
            # 链到根目录会报 "ndk doesn't support sdk version 35"。
            # 已存在且不是软链的东西不动，交给用户判断（可能是 xbuild 自己下载的）。
            x_cache="''${XDG_CACHE_HOME:-$HOME/.cache}/x"
            ndk_sysroot="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
            mkdir -p "$x_cache"
            for pair in "Android.sdk:${xbuildSdk}" "Android.ndk:$ndk_sysroot"; do
              link="$x_cache/''${pair%%:*}"
              want="''${pair#*:}"
              if [ -L "$link" ]; then
                ln -sfn "$want" "$link"
              elif [ ! -e "$link" ]; then
                ln -s "$want" "$link"
              else
                echo "提示：$link 已存在且不是软链，xbuild 会用它而不是 $want。"
              fi
            done
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

          # Android 交叉环境：SDK/NDK 全出自上面的 androidSdk，不读本机 ~/Android/Sdk。
          ANDROID_HOME = androidSdkRoot;
          ANDROID_SDK_ROOT = androidSdkRoot;
          # 交叉时 pkg-config 只做版本探测、不做链接（真正链接在 P3 接 GStreamer
          # 通用包时再配）；没有这面旗子，带横线的 target 三元组会直接拒绝探测。
          PKG_CONFIG_ALLOW_CROSS = "1";
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
