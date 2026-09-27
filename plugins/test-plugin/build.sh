#!/bin/sh
# 重新编译示例插件的 Guest 字节码，产物 guest.wasm 提交进仓库。
# 必须在 nix develop 壳内运行：flake.nix 的 rustToolchain 已加入
# wasm32-unknown-unknown target，壳外没有标准库。
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cargo build --release --target wasm32-unknown-unknown --manifest-path "$here/guest/Cargo.toml"
cp "$here/guest/target/wasm32-unknown-unknown/release/test_plugin.wasm" "$here/guest.wasm"
ls -l "$here/guest.wasm"
