#!/usr/bin/env bash
# Build the macOS release artifacts: one universal .dmg that runs on both
# Apple Silicon and Intel Macs, plus SHA-256 checksums.
#
# Usage: bash scripts/release-macos.sh
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")"
OUT="release"
TRIPLE="universal-apple-darwin"

echo "== BytePet $VERSION (macOS universal) =="

rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm install --frozen-lockfile
pnpm build

pnpm tauri build --target "$TRIPLE" --bundles app,dmg

BUNDLE="target/$TRIPLE/release/bundle"
DMG="$(ls "$BUNDLE"/dmg/*.dmg | head -1)"

rm -rf "$OUT"
mkdir -p "$OUT"
cp "$DMG" "$OUT/"

echo "== verifying dmg =="
hdiutil verify "$OUT/$(basename "$DMG")" >/dev/null

( cd "$OUT" && shasum -a 256 ./* > SHA256SUMS )

echo
echo "Artifacts in $OUT/:"
ls -lh "$OUT"
echo
echo "Next: bash scripts/publish-release.sh $VERSION"
