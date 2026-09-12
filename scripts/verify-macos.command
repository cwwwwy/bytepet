#!/usr/bin/env bash
set -uo pipefail

BYTEPET_SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
bash "$BYTEPET_SCRIPT_DIR/verify-macos.sh"
BYTEPET_EXIT_CODE=$?

printf '\n'
if [[ $BYTEPET_EXIT_CODE -eq 0 ]]; then
  printf 'macOS verification passed.\n'
else
  printf 'macOS verification failed with exit code %s.\n' "$BYTEPET_EXIT_CODE"
fi

read -r -p "Press Return to close..." _
exit "$BYTEPET_EXIT_CODE"
