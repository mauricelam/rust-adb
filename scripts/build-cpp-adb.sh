#!/usr/bin/env bash
set -euo pipefail

# Scripts directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

VERSION="36.0.1"
EXPECTED_SHA256="38e8a84b739480141de0836bf6d581b3339ac7d53d0f7ce8c044a3368c8c2f8f"
URL="https://github.com/nmeum/android-tools/releases/download/${VERSION}/android-tools-${VERSION}.tar.xz"

DL_DIR="${REPO_ROOT}/target/cpp-build/dl"
WORK_DIR="${REPO_ROOT}/target/cpp-build/work"
TARBALL_PATH="${DL_DIR}/android-tools-${VERSION}.tar.xz"
EXTRACT_DIR="${WORK_DIR}/android-tools-${VERSION}"
BUILD_DIR="${EXTRACT_DIR}/build"

PATCHES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --patch)
      if [[ -z "${2:-}" ]]; then
        echo "Error: --patch requires a file argument" >&2
        exit 1
      fi
      PATCHES+=("$(cd "$(dirname "$2")" && pwd)/$(basename "$2")")
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

# Step 1: Download
mkdir -p "${DL_DIR}"
if [[ ! -f "${TARBALL_PATH}" ]]; then
  echo "Downloading android-tools-${VERSION}.tar.xz..." >&2
  curl -sSL "${URL}" -o "${TARBALL_PATH}"
fi

# Step 2: Verify SHA-256
ACTUAL_SHA256="$(sha256sum "${TARBALL_PATH}" | awk '{print $1}')"
if [[ "${ACTUAL_SHA256}" != "${EXPECTED_SHA256}" ]]; then
  echo "Error: SHA-256 mismatch for ${TARBALL_PATH}!" >&2
  echo "Expected: ${EXPECTED_SHA256}" >&2
  echo "Actual:   ${ACTUAL_SHA256}" >&2
  exit 1
fi

# Step 3: Extract fresh copy into WORK_DIR
mkdir -p "${WORK_DIR}"
rm -rf "${EXTRACT_DIR}"
tar -xJf "${TARBALL_PATH}" -C "${WORK_DIR}"

# Step 4: Apply patches inside vendor/adb
if [[ ${#PATCHES[@]} -gt 0 ]]; then
  echo "Applying ${#PATCHES[@]} patch(es) in ${EXTRACT_DIR}/vendor/adb..." >&2
  for patch_file in "${PATCHES[@]}"; do
    echo "Applying patch: ${patch_file}" >&2
    patch -d "${EXTRACT_DIR}/vendor/adb" -p1 < "${patch_file}"
  done
fi

# Step 5: Build
mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

cmake .. -GNinja \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DANDROID_TOOLS_USE_BUNDLED_LIBUSB=ON \
  > /dev/null

ninja adb > /dev/null

# Step 6: Print result binary path
ADB_BIN="${BUILD_DIR}/vendor/adb"
if [[ -f "${ADB_BIN}" ]]; then
  echo "${ADB_BIN}"
else
  echo "Error: ADB binary not found at ${ADB_BIN}" >&2
  exit 1
fi
