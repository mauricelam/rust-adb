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
REF_VERSION_OUTPUT="$("$ADB_REFERENCE" version)"
echo "$REF_VERSION_OUTPUT"

REF_LINE_1="$(echo "$REF_VERSION_OUTPUT" | sed -n '1p')"
REF_LINE_2="$(echo "$REF_VERSION_OUTPUT" | sed -n '2p')"

# Verify first two lines match constants in crates/adb-cli/src/version.rs
if ! grep -qF "VERSION_LINE_1: &str = \"$REF_LINE_1\";" crates/adb-cli/src/version.rs; then
  echo "Error: VERSION_LINE_1 in crates/adb-cli/src/version.rs does not match reference: $REF_LINE_1" >&2
  exit 1
fi

if ! grep -qF "VERSION_LINE_2: &str = \"$REF_LINE_2\";" crates/adb-cli/src/version.rs; then
  echo "Error: VERSION_LINE_2 in crates/adb-cli/src/version.rs does not match reference: $REF_LINE_2" >&2
  exit 1
fi

echo "=== Step 2: Format check ==="
cargo fmt --all -- --check

echo "=== Step 3: Clippy check ==="
cargo clippy --workspace --all-targets -- -D warnings

echo "=== Step 4: Workspace tests ==="
cargo test --workspace

echo "=== Step 5: Compatibility suite ==="
cargo test -p compat

echo "=== All CI checks passed successfully! ==="
