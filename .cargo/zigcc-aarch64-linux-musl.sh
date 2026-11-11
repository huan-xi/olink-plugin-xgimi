#!/bin/sh
# macOS / CI：cargo --target aarch64-unknown-linux-musl 时供 cc-rs / ring 使用。
# 过滤 Rust 的 --target=<rust-triple>，以及 zig 不支持的 linker fix 参数。
args=
for arg in "$@"; do
  case "$arg" in
    --target=aarch64-unknown-linux-musl|--target=*) ;;
    -Wl,--fix-cortex-a53-843419|--fix-cortex-a53-843419) ;;
    *) args="$args $arg" ;;
  esac
done
exec zig cc -target aarch64-linux-musl $args
