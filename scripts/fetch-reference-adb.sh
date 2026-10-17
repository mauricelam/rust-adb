#!/usr/bin/env bash
# Fetches the reference ADB C++ binary matching pinned upstream revision platform-tools-36.0.2 (Build 14143358).
# Uses official platform-tools r36.0.2 binaries since full C++ source build is deferred to task 0.10.
# Idempotent: safe to run multiple times.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_DIR="$REPO_ROOT/target/reference"
ADB_BIN="$TARGET_DIR/platform-tools/adb"

if [ -f "$ADB_BIN" ]; then
    echo "$ADB_BIN"
    exit 0
fi

mkdir -p "$TARGET_DIR"

OS="$(uname -s)"
case "$OS" in
    Linux*)
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-linux.zip"
        ;;
    Darwin*)
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-darwin.zip"
        ;;
    *)
        echo "Unsupported OS: $OS" >&2
        exit 1
        ;;
esac

ZIP_FILE="$TARGET_DIR/platform-tools.zip"

if command -v curl >/dev/null 2>&1; then
    curl -sL "$URL" -o "$ZIP_FILE"
elif command -v wget >/dev/null 2>&1; then
    wget -q "$URL" -O "$ZIP_FILE"
else
    echo "Neither curl nor wget found." >&2
    exit 1
fi

unzip -q -o "$ZIP_FILE" -d "$TARGET_DIR"
rm -f "$ZIP_FILE"
chmod +x "$ADB_BIN"

echo "$ADB_BIN"
