#!/usr/bin/env bash
# Upload everything in release/ to a GitHub Release.
#
# Usage: bash scripts/publish-release.sh 0.2.0
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: $0 <version>" >&2
  exit 1
fi

if [[ ! -d release ]] || [[ -z "$(ls -A release 2>/dev/null)" ]]; then
  echo "release/ is empty - run scripts/release-macos.sh (and the Windows script) first." >&2
  exit 1
fi

TAG="v$VERSION"

if ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "warning: tag $TAG does not exist yet (run scripts/bump-version.sh $VERSION --tag)" >&2
fi

NOTES="$(mktemp)"
{
  echo "## BytePet $VERSION"
  echo
  echo "Download the installer for your platform:"
  echo
  echo "- **macOS** (Apple Silicon + Intel): \`BytePet_${VERSION}_universal.dmg\`"
  echo "  Unsigned build: on first launch use right-click -> Open, or run"
  echo "  \`xattr -dr com.apple.quarantine /Applications/BytePet.app\`."
  echo "- **Windows** (x64): \`BytePet_${VERSION}_x64-setup.exe\`"
  echo "  SmartScreen may warn about an unknown publisher: More info -> Run anyway."
  echo "  WebView2 is preinstalled on Windows 11; Windows 10 downloads it during setup."
  echo
  echo "No toolchain or manual dependency install is required."
  echo
  echo '```'
  cat release/SHA256SUMS 2>/dev/null || echo "(no checksums found)"
  echo '```'
} > "$NOTES"

gh release create "$TAG" release/* \
  --title "BytePet $VERSION" \
  --notes-file "$NOTES"

echo "published $TAG"
