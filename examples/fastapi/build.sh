#!/bin/sh
# Build a ready-to-run copy of the shop in $1: the code and its dependencies.
# Run it where the app runs, so compiled dependencies match the host.
set -eu

out=${1:?usage: build.sh DIRECTORY}
here=$(dirname "$0")
rm -rf "$out"
mkdir -p "$out"
cp "$here/main.py" "$out/"
python3 -m pip install --quiet --disable-pip-version-check --no-compile --target "$out/deps" -r "$here/requirements.txt"
