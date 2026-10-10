#!/usr/bin/env bash
# Windows builds from Linux (what GitHub's release workflow does, adapted to
# this workspace's Rust, which has no prebuilt Windows standard library).
#   tools/windows.sh check    quick compile check of the Windows-only code
#   tools/windows.sh build    TUFFcord .exe files in target/x86_64-pc-windows-gnu/release
# Needs Ubuntu's rustc-1.91/cargo-1.91/rust-1.91-src, gcc-mingw-w64-x86-64,
# g++-mingw-w64-x86-64 and cmake. STD_SRC: the Rust standard library source
# (default ~/.cache/tuffcord/rust-std-src/library; tools/setup.sh prepares it).
# On a Windows PC with Rust installed, just run `cargo build --release`.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH=/usr/lib/rust-1.91/bin:$PATH
export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT=${STD_SRC:-$HOME/.cache/tuffcord/rust-std-src/library}
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=-static -C link-arg=-static-libgcc"
opts=(--release --target x86_64-pc-windows-gnu -Z build-std=std,panic_abort --message-format short
      -p backroom-server --bin backroom-server -p backroom --bin backroom)
case "${1:-}" in
  check) cargo check "${opts[@]}" 2>&1 | grep -E "^(error|warning)|^[^ ]+:[0-9]+:[0-9]+: (error|warning)" || echo "windows check: ok" ;;
  build) cargo build "${opts[@]}" 2>&1 | grep -E "error|warning: unused" || true
         ls -la target/x86_64-pc-windows-gnu/release/*.exe ;;
  *) echo "usage: tools/windows.sh check|build"; exit 2 ;;
esac
