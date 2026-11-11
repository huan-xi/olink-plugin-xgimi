#!/bin/sh
# aarch64 musl 最终链接：丢掉 rustc self-contained CRT，由 zig 注入一份；
# 并过滤 zig/ld.lld 不支持的 Cortex-A53 erratum 参数。
args=
for arg in "$@"; do
  case "$arg" in
    -Wl,--fix-cortex-a53-843419|--fix-cortex-a53-843419) ;;
    */crt1.o|*/crti.o|*/crtbegin.o|*/crtend.o|*/crtn.o|crt1.o|crti.o|crtbegin.o|crtend.o|crtn.o) ;;
    -nostartfiles) ;;
    *) args="$args $arg" ;;
  esac
done
exec zig cc -target aarch64-linux-musl $args
