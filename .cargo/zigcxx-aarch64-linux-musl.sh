#!/bin/sh
args=
for arg in "$@"; do
  case "$arg" in
    --target=aarch64-unknown-linux-musl|--target=*) ;;
    -Wl,--fix-cortex-a53-843419|--fix-cortex-a53-843419) ;;
    *) args="$args $arg" ;;
  esac
done
exec zig c++ -target aarch64-linux-musl $args
