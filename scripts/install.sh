#!/bin/sh
# curl -fsSL https://raw.githubusercontent.com/<org>/claim-engine/main/scripts/install.sh | sh
# Downloads the prebuilt single-file executable for this platform (§12).
set -eu

REPO="${CLAIM_ENGINE_REPO:-your-org/claim-engine}"
VERSION="${CLAIM_ENGINE_VERSION:-latest}"
BIN_DIR="${CLAIM_ENGINE_BIN_DIR:-/usr/local/bin}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$os" in linux) os=linux ;; darwin) os=darwin ;; *) echo "unsupported OS: $os" >&2; exit 1 ;; esac
case "$arch" in x86_64|amd64) arch=x64 ;; arm64|aarch64) arch=arm64 ;; *) echo "unsupported arch: $arch" >&2; exit 1 ;; esac

target="${os}-${arch}"
if [ "$VERSION" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/claim-engine-${target}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/claim-engine-${target}"
fi

echo "Downloading claim-engine (${target}) from ${url}"
tmp="$(mktemp)"
curl -fsSL "$url" -o "$tmp"
chmod +x "$tmp"
mv "$tmp" "${BIN_DIR}/claim-engine"
echo "Installed claim-engine to ${BIN_DIR}/claim-engine"
