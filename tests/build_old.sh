#!/usr/bin/env bash
# Build the Linux server and test bot of an earlier release, for the old/new
# version checks. Prints the folder holding them (use it as OLD_BIN).
#   tests/build_old.sh          the newest release older than this version
#   tests/build_old.sh v0.8.0   a particular one
# Builds are kept in target/compat, so each version is only built once.
set -euo pipefail
cd "$(dirname "$0")/.."
git fetch --quiet --tags origin 2>/dev/null || true
current="v$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
tag="${1:-$(git tag --list 'v*' --sort=-v:refname | grep -vx "$current" | head -1)}"
out="target/compat/$tag"
if [ ! -x "$out/backroom-server" ] || [ ! -x "$out/backroom-bot" ]; then
  src="target/compat/src-$tag"
  rm -rf "$src"
  mkdir -p "$src"
  git archive "$tag" | tar -x -C "$src"
  cargo build --quiet --manifest-path "$src/Cargo.toml" --target-dir target/compat/target \
    -p backroom-server --bin backroom-server -p backroom --bin backroom-bot >&2
  mkdir -p "$out"
  cp target/compat/target/debug/backroom-server target/compat/target/debug/backroom-bot "$out/"
  rm -rf "$src"
fi
echo "$PWD/$out"
