#!/usr/bin/env bash
#
# Fetch reference ADB binary (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2), checked in under original/.
#
# Reference ADB binaries matching this revision are included in binaries/ in the repository
# and populated into target/reference/.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target/reference"

OS="$(uname -s)"
case "${OS}" in
    Linux*)
        SRC_DIR="${REPO_ROOT}/binaries/linux"
        EXE_NAME="adb"
        ;;
    Darwin*)
        SRC_DIR="${REPO_ROOT}/binaries/mac"
        EXE_NAME="adb"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        if [ -d "${REPO_ROOT}/binaries/win" ]; then
            SRC_DIR="${REPO_ROOT}/binaries/win"
        elif [ -d "${REPO_ROOT}/binaries/windows" ]; then
            SRC_DIR="${REPO_ROOT}/binaries/windows"
        else
            SRC_DIR="${REPO_ROOT}/binaries/linux"
        fi
        EXE_NAME="adb.exe"
        if [ ! -f "${SRC_DIR}/${EXE_NAME}" ] && [ -f "${SRC_DIR}/adb" ]; then
            EXE_NAME="adb"
        fi
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

if [ ! -d "${SRC_DIR}" ]; then
    echo "Reference binary directory not found at ${SRC_DIR}" >&2
    exit 1
fi

mkdir -p "${TARGET_DIR}"
cp -r "${SRC_DIR}"/* "${TARGET_DIR}/"

if [ -f "${TARGET_DIR}/${EXE_NAME}" ]; then
    chmod +x "${TARGET_DIR}/${EXE_NAME}"
fi

echo "${ADB_BIN}"
