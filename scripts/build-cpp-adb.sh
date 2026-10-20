#!/usr/bin/env bash
set -euo pipefail

# Script to build C++ adb from source using the android-tools-cmake submodule.
# Accepts optional --patch <file> arguments to apply patches inside vendor/adb.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SUBMODULE_DIR="${REPO_ROOT}/android-tools-cmake"
WORK_DIR="${REPO_ROOT}/target/cpp-build/work"
DL_DIR="${REPO_ROOT}/target/cpp-build/dl"

PATCHES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --patch)
      if [[ -z "${2:-}" ]]; then
        echo "Error: --patch requires a file argument" >&2
        exit 1
      fi
      PATCHES+=("$(realpath "$2")")
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

# Ensure submodules are initialized
git -C "${REPO_ROOT}" submodule update --init --recursive >/dev/null 2>&1 || true

# Helper to clean/reset submodules properly
reset_submodule() {
  local dir="$1"
  if [[ -d "${dir}" ]]; then
    git -C "${dir}" am --abort >/dev/null 2>&1 || true
    git -C "${dir}" reset --hard >/dev/null 2>&1 || true
    git -C "${dir}" clean -fdx >/dev/null 2>&1 || true
  fi
}

# Reset any changes/patches in android-tools-cmake submodules
git -C "${SUBMODULE_DIR}" am --abort >/dev/null 2>&1 || true
reset_submodule "${SUBMODULE_DIR}"
if [[ -d "${SUBMODULE_DIR}/vendor" ]]; then
  for sub in "${SUBMODULE_DIR}"/vendor/*; do
    reset_submodule "${sub}"
  done
fi

# Prepare clean build work directory and download cache dir
mkdir -p "${DL_DIR}"
rm -rf "${WORK_DIR}"
mkdir -p "${WORK_DIR}"

# Configure CMake (this applies android-tools-cmake/patches/* to vendor submodules)
cmake -S "${SUBMODULE_DIR}" -B "${WORK_DIR}" \
  -GNinja \
  -DBUILD_SHARED_LIBS=OFF \
  -DANDROID_TOOLS_USE_BUNDLED_LIBUSB=ON \
  >/dev/null

# Apply custom patches (mutants) if provided
for patch_file in "${PATCHES[@]}"; do
  if [[ ! -f "${patch_file}" ]]; then
    echo "Error: Patch file not found: ${patch_file}" >&2
    exit 1
  fi
  echo "Applying custom patch: ${patch_file}" >&2
  git -C "${SUBMODULE_DIR}/vendor/adb" apply "${patch_file}"
done

# Build adb target
ninja -C "${WORK_DIR}" adb >/dev/null

BUILT_BINARY="${WORK_DIR}/vendor/adb"
if [[ ! -x "${BUILT_BINARY}" ]]; then
  echo "Error: Built adb binary not found or not executable at ${BUILT_BINARY}" >&2
  exit 1
fi

# Print absolute path of built binary
echo "$(realpath "${BUILT_BINARY}")"
