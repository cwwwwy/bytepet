#!/usr/bin/env bash
# Run the Rust-only BytePet verification gates on a local macOS machine.
set -euo pipefail

BYTEPET_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$BYTEPET_ROOT"

step() {
  printf '\n=== %s ===\n' "$1"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'Missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

step "macOS toolchain"
require_command rustc
require_command cargo
require_command xcode-select
require_command xcrun

if ! xcode-select -p >/dev/null 2>&1; then
  printf 'Xcode Command Line Tools are not configured. Run: xcode-select --install\n' >&2
  exit 1
fi

sw_vers
printf 'architecture: %s\n' "$(uname -m)"
rustc --version
cargo --version
xcrun --find clang

step "cargo fmt"
cargo fmt --all -- --check

step "cargo clippy"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test"
cargo test --workspace --locked

step "cargo build (release linker check)"
cargo build --workspace --release --locked

printf '\nAll macOS Rust gates passed.\n'
printf 'Manual window checks remain: docs/MACOS_VERIFICATION.md\n'
