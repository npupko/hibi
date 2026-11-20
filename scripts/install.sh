#!/bin/sh
# Install the hibi binary from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh
#
# Downloads the release tarball for this platform and its .sha256 file,
# checks the checksum, and copies the `hibi` binary into HIBI_BIN_DIR.
# It runs nothing it downloads and changes no shell profile.
#
# Environment:
#   HIBI_VERSION  release to install, such as 5.1.0 or v5.1.0 (default: the latest release)
#   HIBI_BIN_DIR  directory to install into (default: $HOME/.local/bin)
set -eu

REPO="npupko/hibi"
VERSION="${HIBI_VERSION:-latest}"
BIN_DIR="${HIBI_BIN_DIR:-$HOME/.local/bin}"

fail() {
  echo "install.sh: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "this script needs '$1', which was not found"
}

need curl
need tar
need uname
need mktemp

os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "no release build for CPU architecture '$arch'" ;;
esac
case "$os" in
  Darwin) target="${arch}-apple-darwin" ;;
  # The musl builds are statically linked and run on any Linux distribution.
  Linux) target="${arch}-unknown-linux-musl" ;;
  *) fail "no release build for '$os'. On Windows, use the PowerShell installer from the release page." ;;
esac

case "$VERSION" in
  latest) base="https://github.com/${REPO}/releases/latest/download" ;;
  v*) base="https://github.com/${REPO}/releases/download/${VERSION}" ;;
  *) base="https://github.com/${REPO}/releases/download/v${VERSION}" ;;
esac

name="hibi-cli-${target}"
tarball="${name}.tar.xz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading ${base}/${tarball}"
curl --proto '=https' --tlsv1.2 -fsSL "${base}/${tarball}" -o "${tmp}/${tarball}" ||
  fail "download failed: ${base}/${tarball}"
curl --proto '=https' --tlsv1.2 -fsSL "${base}/${tarball}.sha256" -o "${tmp}/${tarball}.sha256" ||
  fail "download failed: ${base}/${tarball}.sha256"

expected="$(awk '{print $1; exit}' "${tmp}/${tarball}.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${tmp}/${tarball}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "${tmp}/${tarball}" | awk '{print $1}')"
else
  fail "this script needs 'sha256sum' or 'shasum' to check the download"
fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] ||
  fail "checksum mismatch for ${tarball}: expected '${expected}', got '${actual}'"

# The tarball holds one directory named like the tarball, with `hibi` inside.
tar -xJf "${tmp}/${tarball}" -C "$tmp" ||
  fail "could not unpack ${tarball}; tar needs xz support (install the 'xz' package)"
[ -f "${tmp}/${name}/hibi" ] || fail "${tarball} does not contain ${name}/hibi"

mkdir -p "$BIN_DIR"
cp "${tmp}/${name}/hibi" "${BIN_DIR}/hibi"
chmod 755 "${BIN_DIR}/hibi"
echo "Installed hibi to ${BIN_DIR}/hibi"

case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *) echo "Add ${BIN_DIR} to your PATH to run hibi." ;;
esac
