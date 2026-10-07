#!/bin/sh
# Package one native CLI binary and its SHA-256 checksum.
set -eu

target=$1
case "$target" in
  x86_64-unknown-linux-musl | aarch64-unknown-linux-musl | x86_64-apple-darwin | aarch64-apple-darwin) ;;
  *) echo 'Unsupported package target.' >&2; exit 1 ;;
esac
asset="ssp-${target}.tar.gz"
mkdir -p dist
package_dir=$(mktemp -d "${TMPDIR:-/tmp}/statespace-package.XXXXXX")
trap 'rm -rf "$package_dir"' EXIT HUP INT TERM
cp "target/${target}/release/ssp" "${package_dir}/ssp"
cp LICENSE "${package_dir}/LICENSE"
tar -C "$package_dir" -czf "dist/${asset}" ssp LICENSE
if command -v sha256sum >/dev/null 2>&1; then
  (cd dist && sha256sum "$asset" > "${asset}.sha256")
else
  (cd dist && shasum -a 256 "$asset" > "${asset}.sha256")
fi
