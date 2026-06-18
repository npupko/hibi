#!/bin/sh
# curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh
# Downloads the prebuilt single-file executable for this platform (§12).
set -eu

REPO="${HIBI_REPO:-npupko/hibi}"
VERSION="${HIBI_VERSION:-latest}"
BIN_DIR="${HIBI_BIN_DIR:-/usr/local/bin}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$os" in linux) os=linux ;; darwin) os=darwin ;; *) echo "unsupported OS: $os" >&2; exit 1 ;; esac
case "$arch" in x86_64|amd64) arch=x64 ;; arm64|aarch64) arch=arm64 ;; *) echo "unsupported arch: $arch" >&2; exit 1 ;; esac

target="${os}-${arch}"
if [ "$VERSION" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/hibi-${target}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/hibi-${target}"
fi

echo "Downloading hibi (${target}) from ${url}"
tmp="$(mktemp)"
curl -fsSL "$url" -o "$tmp"
chmod +x "$tmp"
mv "$tmp" "${BIN_DIR}/hibi"
echo "Installed hibi to ${BIN_DIR}/hibi"
