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
if [ ! -x "$out/TUFFcord-server" ] || [ ! -x "$out/tuffcord-bot" ]; then
  src="target/compat/src-$tag"
  rm -rf "$src"
  mkdir -p "$src"
  git archive "$tag" | tar -x -C "$src"
  # Releases before 0.8.3 named the packages and programs backroom.
  if grep -q '^name = "backroom-server"' "$src/server/Cargo.toml"; then
    pkg=(-p backroom-server --bin backroom-server -p backroom --bin backroom-bot); srv=backroom-server; bot=backroom-bot
  else
    pkg=(-p tuffcord-server --bin TUFFcord-server -p tuffcord --bin tuffcord-bot); srv=TUFFcord-server; bot=tuffcord-bot
  fi
  cargo build --quiet --manifest-path "$src/Cargo.toml" --target-dir target/compat/target "${pkg[@]}" >&2
  mkdir -p "$out"
  cp "target/compat/target/debug/$srv" "$out/TUFFcord-server"
  cp "target/compat/target/debug/$bot" "$out/tuffcord-bot"
  rm -rf "$src"
fi
echo "$PWD/$out"
