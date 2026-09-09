#!/usr/bin/env bash
# Mirrors .github/workflows/ci.yml on a local macOS machine.
# Usage: bash scripts/verify-macos.sh
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n=== %s ===\n' "$1"; }

step "toolchain"
rustc --version
node --version
pnpm --version

step "pnpm install --frozen-lockfile"
pnpm install --frozen-lockfile

step "pnpm build (tsc + vite)"
pnpm build

step "cargo test --workspace"
cargo test --workspace

step "cargo clippy -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo fmt --check"
cargo fmt --all -- --check

step "pnpm test (vitest)"
pnpm test

step "pnpm tauri build --bundles app"
pnpm tauri build --bundles app

printf '\nAll gates passed. Bundle: target/release/bundle/macos/BytePet.app\n'
