#!/usr/bin/env bash
# Fetch / prepare reference C++ adb binary matching pinned platform-tools release (36.0.2 / Build 14143358).
# Copies prebuilt reference adb binary to target/reference/platform-tools/ for the host OS (linux/mac).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

DEST_DIR="${REPO_ROOT}/target/reference/platform-tools"
mkdir -p "${DEST_DIR}"

OS="$(uname -s)"
case "${OS}" in
  Linux*)     OS_DIR="linux"; EXE_NAME="adb" ;;
  Darwin*)    OS_DIR="mac";   EXE_NAME="adb" ;;
  MINGW*|MSYS*|CYGWIN*) OS_DIR="windows"; EXE_NAME="adb.exe" ;;
  *)          echo "Unsupported OS: ${OS}" >&2; exit 1 ;;
esac

SRC_DIR="${REPO_ROOT}/binaries/${OS_DIR}"
if [[ ! -d "${SRC_DIR}" ]]; then
  echo "Error: Prebuilt binaries directory ${SRC_DIR} does not exist." >&2
  exit 1
fi

cp -r "${SRC_DIR}"/* "${DEST_DIR}/"
chmod +x "${DEST_DIR}/${EXE_NAME}" 2>/dev/null || true

echo "${DEST_DIR}/${EXE_NAME}"
