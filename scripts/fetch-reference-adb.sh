#!/usr/bin/env bash
#
# Fetch reference ADB binary (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2), checked in under original/.
#
# Reference ADB binaries are copied from binaries/ in the repository if present,
# or downloaded from official platform-tools releases into target/reference/.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target/reference"

OS="$(uname -s)"
case "${OS}" in
    Linux*)
        SRC_DIR="${REPO_ROOT}/binaries/linux"
        EXE_NAME="adb"
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-linux.zip"
        ;;
    Darwin*)
        SRC_DIR="${REPO_ROOT}/binaries/mac"
        EXE_NAME="adb"
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-darwin.zip"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        if [ -d "${REPO_ROOT}/binaries/win" ]; then
            SRC_DIR="${REPO_ROOT}/binaries/win"
        elif [ -d "${REPO_ROOT}/binaries/windows" ]; then
            SRC_DIR="${REPO_ROOT}/binaries/windows"
        else
            SRC_DIR=""
        fi
        EXE_NAME="adb.exe"
        URL="https://dl.google.com/android/repository/platform-tools_r36.0.2-win.zip"
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

if [ -n "${SRC_DIR:-}" ] && [ -d "${SRC_DIR}" ]; then
    cp -r "${SRC_DIR}"/* "${TARGET_DIR}/"
else
    TEMP_ZIP="${REPO_ROOT}/temp_reference.zip"
    TEMP_EXTRACT="${REPO_ROOT}/temp_reference_extract"

    echo "Downloading reference ADB from ${URL}..." >&2
    curl -sSL -o "${TEMP_ZIP}" "${URL}"

    mkdir -p "${TEMP_EXTRACT}"
    unzip -o -q "${TEMP_ZIP}" -d "${TEMP_EXTRACT}"

    cp -r "${TEMP_EXTRACT}/platform-tools/"* "${TARGET_DIR}/"
    rm -rf "${TEMP_EXTRACT}" "${TEMP_ZIP}"
fi

if [ -f "${TARGET_DIR}/${EXE_NAME}" ]; then
    chmod +x "${TARGET_DIR}/${EXE_NAME}"
fi

if command -v powershell.exe >/dev/null 2>&1; then
    powershell.exe -Command "Get-ChildItem '${TARGET_DIR}' -Recurse | Unblock-File" 2>/dev/null || true
fi

echo "${ADB_BIN}"
