#!/bin/sh

set -eu

repository=${STATESPACE_REPOSITORY:-statespace-tech/cli}
version=${STATESPACE_VERSION:-main}
install_dir=${STATESPACE_INSTALL_DIR:-"${HOME}/.local/bin"}

temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/statespace-install.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-musl ;;
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
elif [ "$version" = main ]; then
  curl -fsSL "https://statespace.com/cli/latest.txt" -o "${temporary_dir}/latest"
  commit=$(cat "${temporary_dir}/latest")
  case "$commit" in
    *[!a-f0-9]* | "") echo "ssp: invalid main build manifest" >&2; exit 1 ;;
  esac
  if [ "${#commit}" -ne 40 ]; then
    echo "ssp: invalid main build commit" >&2
    exit 1
  fi
  download_root="https://statespace.com/cli/${commit}"
elif [ "$version" = latest ]; then
  download_root="https://github.com/${repository}/releases/latest/download"
else
  case "$version" in
    v*) tag=$version ;;
    *) tag="v${version}" ;;
  esac
  download_root="https://github.com/${repository}/releases/download/${tag}"
fi


curl -fsSL \
  "${download_root}/${archive}" -o "${temporary_dir}/${archive}"
curl -fsSL \
  "${download_root}/${archive}.sha256" -o "${temporary_dir}/${archive}.sha256"

expected=$(awk 'NR == 1 {print $1}' "${temporary_dir}/${archive}.sha256")
case "$expected" in
  *[!a-f0-9]* | "") echo "ssp: invalid checksum" >&2; exit 1 ;;
esac
if [ "${#expected}" -ne 64 ]; then
  echo "ssp: invalid checksum length" >&2
  exit 1
fi
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
