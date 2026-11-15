#!/bin/sh
# Installs prinia from its latest GitHub release.
#   curl -fsSL https://github.com/drumandbytes/prinia/releases/latest/download/install.sh | sh
# PRINIA_VERSION=v0.1.0 pins a release; PRINIA_INSTALL_DIR changes the target
# (default ~/.local/bin); PRINIA_DOWNLOAD_URL replaces the release download URL (mirrors, tests).
set -eu

repo="drumandbytes/prinia"
dir="${PRINIA_INSTALL_DIR:-$HOME/.local/bin}"
version="${PRINIA_VERSION:-latest}"

fail() {
  printf 'install: %s\n' "$*" >&2
  exit 1
}

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) fail "unsupported OS $(uname -s); macOS and Linux only (Windows is not supported yet)" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=amd64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) fail "unsupported architecture $(uname -m)" ;;
esac
asset="prinia_${os}_${arch}.tar.gz"

if [ -n "${PRINIA_DOWNLOAD_URL:-}" ]; then
  base="$PRINIA_DOWNLOAD_URL"
elif [ "$version" = latest ]; then
  base="https://github.com/$repo/releases/latest/download"
else
  base="https://github.com/$repo/releases/download/$version"
fi

command -v curl >/dev/null 2>&1 || fail "curl is required"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$base/$asset" -o "$tmp/$asset" || fail "download failed: $base/$asset"
curl -fsSL "$base/checksums.txt" -o "$tmp/checksums.txt" || fail "download failed: $base/checksums.txt"

want=$(awk -v f="$asset" '$2 == f { print $1 }' "$tmp/checksums.txt")
[ -n "$want" ] || fail "no checksum listed for $asset"
if command -v sha256sum >/dev/null 2>&1; then
  got=$(sha256sum "$tmp/$asset" | awk '{ print $1 }')
else
  got=$(shasum -a 256 "$tmp/$asset" | awk '{ print $1 }')
fi
[ "$got" = "$want" ] || fail "checksum mismatch for $asset; not installing"

tar -xzf "$tmp/$asset" -C "$tmp" prinia
mkdir -p "$dir"
install -m 755 "$tmp/prinia" "$dir/prinia"

echo "Installed $("$dir/prinia" --version) to $dir/prinia"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH." ;;
esac
echo 'Next, set an API key (for example: export ANTHROPIC_API_KEY=...) and try: prinia "hello"'
echo "Setup and usage: https://github.com/drumandbytes/prinia#readme"
