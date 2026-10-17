#!/usr/bin/env bash
#
# Fetch reference ADB binary (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2).
#
# Note: Building full C++ ADB from source requires the full Android source tree / Soong build system.
# Therefore, official platform-tools binaries corresponding to the pinned revision are fetched.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target/reference"

OS="$(uname -s)"
case "${OS}" in
    Linux*)
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-linux.zip"
        EXE_NAME="adb"
        ;;
    Darwin*)
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-darwin.zip"
        EXE_NAME="adb"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-win.zip"
        EXE_NAME="adb.exe"
        ;;
    *)
        echo "Unsupported OS: ${OS}" >&2
        exit 1
        ;;
esac

ADB_BIN="${TARGET_DIR}/${EXE_NAME}"

if [ -f "${ADB_BIN}" ]; then
    echo "${ADB_BIN}"
    exit 0
fi

mkdir -p "${TARGET_DIR}"
TEMP_ZIP="${TARGET_DIR}/platform-tools.zip"
TEMP_EXTRACT="${TARGET_DIR}/temp_extract"

echo "Downloading reference ADB from ${URL}..." >&2
curl -sSL -o "${TEMP_ZIP}" "${URL}"

mkdir -p "${TEMP_EXTRACT}"
unzip -o -q "${TEMP_ZIP}" -d "${TEMP_EXTRACT}"

mv "${TEMP_EXTRACT}/platform-tools/"* "${TARGET_DIR}/"
rm -rf "${TEMP_EXTRACT}" "${TEMP_ZIP}"

chmod +x "${ADB_BIN}"

echo "${ADB_BIN}"
