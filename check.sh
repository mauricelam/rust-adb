#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

echo "=== Running local CI check script ==="

# Auto-detect OS and set ADB_REFERENCE if not already set
if [[ -z "${ADB_REFERENCE:-}" ]]; then
  OS="$(uname -s)"
  case "$OS" in
    Linux*)
      chmod +x binaries/linux/adb
      export ADB_REFERENCE="$ROOT_DIR/binaries/linux/adb"
      ;;
    Darwin*)
      chmod +x binaries/mac/adb
      export ADB_REFERENCE="$ROOT_DIR/binaries/mac/adb"
      ;;
    MINGW*|MSYS*|CYGWIN*)
      export ADB_REFERENCE="$ROOT_DIR/binaries/windows/adb.exe"
      ;;
    *)
      echo "Unsupported OS: $OS" >&2
      exit 1
      ;;
  esac
fi

export ADB_CLIENT="${ADB_CLIENT:-$ADB_REFERENCE}"
export ADB_SERVER_BIN="${ADB_SERVER_BIN:-$ADB_REFERENCE}"

echo "Using ADB_REFERENCE: $ADB_REFERENCE"

echo "=== Step 1: Verifying reference ADB version ==="
"$ADB_REFERENCE" version

echo "=== Step 2: Format check ==="
cargo fmt --all -- --check

echo "=== Step 3: Clippy check ==="
cargo clippy --workspace --all-targets -- -D warnings

echo "=== Step 4: Workspace tests ==="
cargo nextest run --workspace

echo "=== Step 5: Compatibility suite ==="
cargo nextest run -p compat

echo "=== All CI checks passed successfully! ==="
