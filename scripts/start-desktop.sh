#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "Cirra desktop currently requires macOS Keychain." >&2
  exit 1
fi
npm run build:ui
# Assets are bundled, including in development. No HTTP development server or
# croaker-local process is involved. Cargo run is intentionally unsigned.
exec cargo run -p cirra-desktop "$@"
