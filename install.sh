#!/bin/sh

set -eu

repository=${STATESPACE_REPOSITORY:-statespace-tech/cli}
version=${STATESPACE_VERSION:-latest}
install_dir=${STATESPACE_INSTALL_DIR:-"${HOME}/.local/bin"}

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-gnu ;;
  *)
    echo "ssp: unsupported operating system: $(uname -s)" >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *)
    echo "ssp: unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

target="${arch}-${os}"
archive="ssp-${target}.tar.gz"

if [ -n "${STATESPACE_DOWNLOAD_ROOT:-}" ]; then
  download_root=${STATESPACE_DOWNLOAD_ROOT%/}
elif [ "$version" = latest ]; then
  download_root="https://github.com/${repository}/releases/latest/download"
else
  case "$version" in
    v*) tag=$version ;;
    *) tag="v${version}" ;;
  esac
  download_root="https://github.com/${repository}/releases/download/${tag}"
fi

temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/statespace-install.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM

curl -fsSL \
  "${download_root}/${archive}" -o "${temporary_dir}/${archive}"
curl -fsSL \
  "${download_root}/${archive}.sha256" -o "${temporary_dir}/${archive}.sha256"

expected=$(awk '{print $1}' "${temporary_dir}/${archive}.sha256")
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "${temporary_dir}/${archive}" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "${temporary_dir}/${archive}" | awk '{print $1}')
else
  echo "ssp: sha256sum or shasum is required" >&2
  exit 1
fi

if [ "$actual" != "$expected" ]; then
  echo "ssp: checksum verification failed" >&2
  exit 1
fi

tar -xzf "${temporary_dir}/${archive}" -C "$temporary_dir"
mkdir -p "$install_dir"
install -m 0755 "${temporary_dir}/ssp" "${install_dir}/ssp"

echo "Installed ssp to ${install_dir}/ssp"
case ":${PATH}:" in
  *":${install_dir}:"*) ;;
  *) echo "Add ${install_dir} to PATH to run ssp." ;;
esac
