#!/usr/bin/env bash
# Set the BytePet version in every place it is stored.
#
# Usage:
#   bash scripts/bump-version.sh 0.2.0          # edit files only
#   bash scripts/bump-version.sh 0.2.0 --tag    # edit + commit + tag v0.2.0
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-}"
TAG="${2:-}"

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 <major.minor.patch> [--tag]" >&2
  exit 1
fi

python3 - "$VERSION" <<'PY'
import json, re, sys
version = sys.argv[1]

# 1. Tauri config (the source of truth for bundle naming).
path = "src-tauri/tauri.conf.json"
data = json.load(open(path))
data["version"] = version
with open(path, "w") as fh:
    json.dump(data, fh, indent=2, ensure_ascii=False)
    fh.write("\n")

# 2. Cargo workspace version.
path = "Cargo.toml"
text = open(path).read()
text, n = re.subn(
    r'(?m)^(\[workspace\.package\]\n(?:.*\n)*?version\s*=\s*")[^"]+(")',
    rf"\g<1>{version}\g<2>",
    text,
    count=1,
)
assert n == 1, "workspace version not found in Cargo.toml"
open(path, "w").write(text)

# 3. package.json.
path = "package.json"
data = json.load(open(path))
data["version"] = version
with open(path, "w") as fh:
    json.dump(data, fh, indent=2, ensure_ascii=False)
    fh.write("\n")

print(f"version -> {version}")
PY

echo "files updated. The next cargo build refreshes Cargo.lock; include it in the commit."

if [[ "$TAG" == "--tag" ]]; then
  git add -A
  git commit -m "release: v$VERSION"
  git tag -a "v$VERSION" -m "BytePet v$VERSION"
  echo "tagged v$VERSION. Push with: git push origin main --tags"
fi
