#!/usr/bin/env bash
# 打包 .olinkpkg。默认打本机平台；交叉打包示例：
#   ./scripts/pack.sh
#   ./scripts/pack.sh linux aarch64
#   ./scripts/pack.sh dist/xgimi.olinkpkg linux aarch64
#   TARGET=aarch64-unknown-linux-musl ./scripts/pack.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

normalize_arch() {
  case "$1" in
    x86_64|amd64) echo x86_64 ;;
    aarch64|arm64) echo aarch64 ;;
    *) echo "$1" ;;
  esac
}

host_os="$(uname -s | tr '[:upper:]' '[:lower:]')"
host_arch="$(normalize_arch "$(uname -m)")"

# 参数： [out] [os] [arch]  或  [os] [arch]
OUT_ARG=""
if [[ "${1:-}" == *.olinkpkg || "${1:-}" == */* || "${1:-}" == .* ]]; then
  OUT_ARG="${1:-}"
  OS="${2:-${PACK_OS:-$host_os}}"
  ARCH="$(normalize_arch "${3:-${PACK_ARCH:-$host_arch}}")"
elif [[ -n "${1:-}" ]]; then
  OS="${1:-${PACK_OS:-$host_os}}"
  ARCH="$(normalize_arch "${2:-${PACK_ARCH:-$host_arch}}")"
else
  OS="${PACK_OS:-$host_os}"
  ARCH="$(normalize_arch "${PACK_ARCH:-$host_arch}")"
fi

# Host 用 std::env::consts::{OS,ARCH} → bin/linux-aarch64/plugin
# linux 交叉统一 musl（与板子静态链接一致）
triple_for() {
  local os="$1" arch="$2"
  case "${os}-${arch}" in
    linux-aarch64) echo aarch64-unknown-linux-musl ;;
    linux-x86_64) echo x86_64-unknown-linux-musl ;;
    darwin-aarch64) echo aarch64-apple-darwin ;;
    darwin-x86_64) echo x86_64-apple-darwin ;;
    *)
      echo "unsupported pack platform: ${os}-${arch}" >&2
      exit 1
      ;;
  esac
}

TRIPLE="${TARGET:-$(triple_for "$OS" "$ARCH")}"
# 若只给了 TARGET，从 triple 反推包内目录名
if [[ -n "${TARGET:-}" && -z "${1:-}" && -z "${PACK_OS:-}" ]]; then
  case "$TRIPLE" in
    *-unknown-linux-musl|*-unknown-linux-gnu)
      OS=linux
      ARCH="$(normalize_arch "${TRIPLE%%-*}")"
      ;;
    *-apple-darwin)
      OS=darwin
      ARCH="$(normalize_arch "${TRIPLE%%-*}")"
      ;;
  esac
fi

VER="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/plugin.json" | head -1)"
VER="${VER:-0.0.0}"
PLATFORM="${OS}-${ARCH}"

# 默认 / 补全文件名里的架构
if [[ -z "$OUT_ARG" ]]; then
  OUT="$ROOT/dist/xgimi-${VER}-${PLATFORM}.olinkpkg"
else
  OUT="$OUT_ARG"
  [[ "$OUT" = /* ]] || OUT="$ROOT/$OUT"
  base="$(basename "$OUT")"
  if [[ "$base" != *"${PLATFORM}"* ]]; then
    dir="$(dirname "$OUT")"
    stem="${base%.olinkpkg}"
    OUT="${dir}/${stem}-${PLATFORM}.olinkpkg"
  fi
fi

cd "$ROOT"
echo "pack target: ${PLATFORM} (cargo --target ${TRIPLE})"
rustup target add "$TRIPLE" >/dev/null 2>&1 || true
cargo build --release --target "$TRIPLE"
BIN="$ROOT/target/${TRIPLE}/release/olink-plugin-xgimi"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/bin/${PLATFORM}" "$STAGE/ui/icons"
cp "$BIN" "$STAGE/bin/${PLATFORM}/plugin"
chmod 755 "$STAGE/bin/${PLATFORM}/plugin"
cp "$ROOT/plugin.json" "$STAGE/plugin.json"
cp "$ROOT/ui/entry.js" "$STAGE/ui/entry.js"
cp "$ROOT/ui/entry.css" "$STAGE/ui/entry.css"
cp "$ROOT/ui/icons/icon.svg" "$STAGE/ui/icons/icon.svg"

mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
( cd "$STAGE" && zip -qr "$OUT" plugin.json bin ui )
echo "packed: $OUT"
unzip -l "$OUT"
