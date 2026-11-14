#!/bin/sh
# Installs atclaude from its latest GitHub release.
#   curl -fsSL https://atclaude.drumandbytes.dev/install.sh | sh
# ATCLAUDE_VERSION=v0.1.0 pins a release; ATCLAUDE_INSTALL_DIR changes the target
# (default ~/.local/bin); ATCLAUDE_DOWNLOAD_URL replaces the release download URL (mirrors, tests).
set -eu

repo="drumandbytes/atclaude"
dir="${ATCLAUDE_INSTALL_DIR:-$HOME/.local/bin}"
version="${ATCLAUDE_VERSION:-latest}"

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
asset="atclaude_${os}_${arch}.tar.gz"

if [ -n "${ATCLAUDE_DOWNLOAD_URL:-}" ]; then
  base="$ATCLAUDE_DOWNLOAD_URL"
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

tar -xzf "$tmp/$asset" -C "$tmp" atclaude
mkdir -p "$dir"
install -m 755 "$tmp/atclaude" "$dir/atclaude"

echo "Installed $("$dir/atclaude" --version) to $dir/atclaude"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH." ;;
esac
# shellcheck disable=SC2016 # the $(...) is meant literally
echo 'Next, add the shell integration to your startup file, e.g. for zsh: eval "$(atclaude init zsh)"'
echo "Other shells and the full setup: https://atclaude.drumandbytes.dev"
