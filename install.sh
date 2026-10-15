#!/bin/sh
# squish installer for Linux and macOS.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/idarciooliveira/squish/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --to ~/.local/bin --version v0.1.0
#
# Env vars: SQUISH_VERSION (tag like v0.1.0, or "latest"), SQUISH_BINDIR.
set -eu

REPO="idarciooliveira/squish"
BIN="squish"
BINDIR="${SQUISH_BINDIR:-$HOME/.local/bin}"
VERSION="${SQUISH_VERSION:-latest}"

usage() {
  echo "usage: install.sh [--to DIR] [--version TAG] [--help]" >&2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --to) BINDIR="$2"; shift 2 ;;
    --prefix) BINDIR="$2/bin"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage; exit 2 ;;
  esac
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "install.sh: need '$1' in PATH" >&2; exit 1; }; }
need curl
need tar
need uname
need mktemp

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Linux/x86_64)   target="x86_64-unknown-linux-musl" ;;
  Linux/aarch64|Linux/arm64) target="aarch64-unknown-linux-musl" ;;
  Darwin/x86_64)  target="x86_64-apple-darwin" ;;
  Darwin/arm64|Darwin/aarch64) target="aarch64-apple-darwin" ;;
  *) echo "install.sh: unsupported platform $os/$arch." >&2
     echo "Windows: download the zip from https://github.com/$REPO/releases" >&2
     exit 1 ;;
esac

if [ "$VERSION" = "latest" ]; then
  need grep
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep -m1 '"tag_name"' | cut -d'"' -f4)"
  [ -n "$VERSION" ] || { echo "install.sh: could not resolve latest release" >&2; exit 1; }
fi

url="https://github.com/$REPO/releases/download/$VERSION/squish-$target.tar.gz"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT INT TERM
echo "downloading $url"
curl -fsSL "$url" -o "$tmpdir/squish.tar.gz"

# Verify checksum when possible (non-fatal if the sums file is missing).
if command -v sha256sum >/dev/null 2>&1; then
  if curl -fsSL "https://github.com/$REPO/releases/download/$VERSION/SHA256SUMS" -o "$tmpdir/SHA256SUMS" 2>/dev/null; then
    if grep -F "squish-$target.tar.gz" "$tmpdir/SHA256SUMS" > "$tmpdir/SHA256SUMS.one" 2>/dev/null; then
      (cd "$tmpdir" && cp squish.tar.gz "squish-$target.tar.gz" && sha256sum -c --status "SHA256SUMS.one") \
        && echo "checksum ok" || echo "install.sh: warning: checksum check failed" >&2
      rm -f "$tmpdir/squish-$target.tar.gz" "$tmpdir/SHA256SUMS.one"
    fi
  fi
fi

tar -xzf "$tmpdir/squish.tar.gz" -C "$tmpdir"
mkdir -p "$BINDIR"
mv "$tmpdir/$BIN" "$BINDIR/$BIN"
chmod +x "$BINDIR/$BIN"
echo "installed $BINDIR/$BIN ($VERSION, $target)"
"$BINDIR/$BIN" --version 2>/dev/null || "$BINDIR/$BIN" --help >/dev/null

case ":$PATH:" in
  *":$BINDIR:"*) ;;
  *) echo "note: $BINDIR is not in PATH. Add: export PATH=\"$BINDIR:\$PATH\"" >&2 ;;
esac
