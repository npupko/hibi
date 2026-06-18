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
  base="https://github.com/${REPO}/releases/latest/download"
else
  base="https://github.com/${REPO}/releases/download/${VERSION}"
fi
url="${base}/hibi-${target}"

echo "Downloading hibi (${target}) from ${url}"
tmp="$(mktemp)"
sums="$(mktemp)"
trap 'rm -f "$tmp" "$sums"' EXIT
curl -fsSL "$url" -o "$tmp"

# Verify the download against the release's published SHA256SUMS.txt.
if curl -fsSL "${base}/SHA256SUMS.txt" -o "$sums"; then
  expected="$(awk -v f="hibi-${target}" '$2==f {print $1}' "$sums")"
  if [ -n "$expected" ]; then
    if command -v sha256sum >/dev/null 2>&1; then
      actual="$(sha256sum "$tmp" | awk '{print $1}')"
    else
      actual="$(shasum -a 256 "$tmp" | awk '{print $1}')"
    fi
    if [ "$expected" != "$actual" ]; then
      echo "checksum mismatch for hibi-${target}" >&2
      echo "  expected: $expected" >&2
      echo "  actual:   $actual" >&2
      exit 1
    fi
    echo "Checksum verified."
  else
    echo "warning: no checksum entry for hibi-${target}; skipping verification" >&2
  fi
else
  echo "warning: could not fetch SHA256SUMS.txt; skipping checksum verification" >&2
fi

chmod +x "$tmp"
mv "$tmp" "${BIN_DIR}/hibi"
trap - EXIT
rm -f "$sums"
echo "Installed hibi to ${BIN_DIR}/hibi"
