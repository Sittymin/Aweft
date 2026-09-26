#!/usr/bin/env bash
# 给 Android 编 libass 的三个 C 依赖（freetype / fribidi / harfbuzz），静态库。
# fontconfig 不编：aweft-libass 的 build.rs 在 Android 上关 fontconfig
#（见 build.rs 的 ANDROID 说明），字体走内嵌 + 系统目录，后续再补。
#
# 用法（nix develop 里）：bash aweft-libass/android-deps.sh
# 产物：aweft-libass/third_party/android-deps/{lib,include,share/pkgconfig}
# （该目录已 gitignore，换机器重跑本脚本即可）。
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/third_party/android-deps"
BUILD="$HERE/third_party/android-deps-build"
API=26

: "${ANDROID_NDK_ROOT:?nix develop 进壳后才有 ANDROID_NDK_ROOT}"
NDK_BIN="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin"
SYSROOT="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
CC="$NDK_BIN/aarch64-linux-android${API}-clang"
CXX="$NDK_BIN/aarch64-linux-android${API}-clang++"
AR="$NDK_BIN/llvm-ar"
STRIP="$NDK_BIN/llvm-strip"

mkdir -p "$OUT" "$BUILD"
export PKG_CONFIG_PATH="$OUT/share/pkgconfig:$OUT/lib/pkgconfig"
export TARGET_PKG_CONFIG_PATH="$PKG_CONFIG_PATH"
export PKG_CONFIG_ALLOW_CROSS=1

# 交叉文件里的 @槽位@ 填上 NDK 实际路径。
sed -e "s|@CC@|$CC|" -e "s|@CXX@|$CXX|" -e "s|@AR@|$AR|" \
    -e "s|@STRIP@|$STRIP|" -e "s|@SYSROOT@|$SYSROOT|" \
    -e "s|@PKGCONFIG_DIR@|$PKG_CONFIG_PATH|" \
    "$HERE/android-cross.ini.in" > "$BUILD/android-cross.ini"

fetch() { # $1=url $2=dir
  if [ ! -d "$BUILD/$2" ]; then
    curl -sSL "$1" -o "$BUILD/$(basename "$1")"
    tar -xf "$BUILD/$(basename "$1")" -C "$BUILD"
  fi
}

meson_build() { # $1=srcdir $2=extra opts...
  local src="$1"; shift
  # 幂等：build.ninja 已存在说明编过，直接跳过（交叉文件改了就手动删对应目录重跑）。
  if [ -f "$BUILD/build-$src/build.ninja" ]; then
    echo "skip $src（已编过）" >&2
    return 0
  fi
  meson setup "$BUILD/build-$src" "$BUILD/$src" \
    --cross-file "$BUILD/android-cross.ini" \
    --buildtype=release --default-library=static --prefer-static \
    --prefix="$OUT" "$@" >&2
  ninja -C "$BUILD/build-$src" >&2
  ninja -C "$BUILD/build-$src" install >&2
}

fetch "https://download.savannah.nongnu.org/releases/freetype/freetype-2.13.3.tar.gz" "freetype-2.13.3"
meson_build "freetype-2.13.3" -Dzlib=disabled -Dbzip2=disabled -Dpng=disabled -Dharfbuzz=disabled -Dbrotli=disabled

fetch "https://github.com/fribidi/fribidi/releases/download/v1.0.16/fribidi-1.0.16.tar.xz" "fribidi-1.0.16"
meson_build "fribidi-1.0.16" -Ddocs=false -Dtests=false

fetch "https://github.com/harfbuzz/harfbuzz/releases/download/11.3.3/harfbuzz-11.3.3.tar.xz" "harfbuzz-11.3.3"
meson_build "harfbuzz-11.3.3" -Dglib=disabled -Dgobject=disabled -Dcairo=disabled -Dicu=disabled -Dtests=disabled -Ddocs=disabled

echo "OK: $OUT"
ls "$OUT/lib" | head
