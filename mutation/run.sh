#!/usr/bin/env bash
set -euo pipefail

# Script to run mutation testing against C++ ADB source code.
# Builds ADB with each patch in mutation/mutants/, executes the compat test suite,
# and records whether each mutant was killed or survived in mutation/results.csv.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

MUTANTS_DIR="${SCRIPT_DIR}/mutants"
RESULTS_FILE="${SCRIPT_DIR}/results.csv"

if [[ ! -d "${MUTANTS_DIR}" ]]; then
  echo "Error: Mutants directory not found at ${MUTANTS_DIR}" >&2
  exit 1
fi

echo "patch,status" > "${RESULTS_FILE}"

# Find all patch files in mutants/ sorted
PATCH_FILES=($(find "${MUTANTS_DIR}" -maxdepth 1 -name "*.patch" | sort))

if [[ ${#PATCH_FILES[@]} -eq 0 ]]; then
  echo "No patch files found in ${MUTANTS_DIR}" >&2
  exit 0
fi

TEST_RUNNER="cargo test -p compat"
if cargo nextest --version >/dev/null 2>&1; then
  TEST_RUNNER="cargo nextest run -p compat"
fi

for patch_file in "${PATCH_FILES[@]}"; do
  patch_name="$(basename "${patch_file}")"
  echo "=========================================="
  echo "Running mutation test for: ${patch_name}"
  echo "=========================================="

  # Build adb with the patch applied
  BUILT_ADB="$("${REPO_ROOT}/scripts/build-cpp-adb.sh" --patch "${patch_file}")"

  # Determine target USB backend:
  # Mutants targeting libusb code run under ADB_LIBUSB=1; others default to 0
  TARGET_LIBUSB="0"
  if grep -qi "usb_libusb\|libusb" "${patch_file}"; then
    TARGET_LIBUSB="1"
  fi

  # Start an isolated server briefly to verify and record the active USB backend
  CHECK_PORT=50370
  ADB_LIBUSB="${TARGET_LIBUSB}" "${BUILT_ADB}" -P "${CHECK_PORT}" start-server >/dev/null 2>&1 || true
  REPORTED_FEATURES="$("${BUILT_ADB}" -P "${CHECK_PORT}" host-features 2>/dev/null || true)"
  "${BUILT_ADB}" -P "${CHECK_PORT}" kill-server >/dev/null 2>&1 || true

  HAS_LIBUSB="0"
  if echo "${REPORTED_FEATURES}" | grep -q "libusb"; then
    HAS_LIBUSB="1"
  fi

  REPORTED_BACKEND="native"
  if [[ "${HAS_LIBUSB}" == "1" ]]; then
    REPORTED_BACKEND="libusb"
  fi

  PINNED_BACKEND="native"
  if [[ "${TARGET_LIBUSB}" == "1" ]]; then
    PINNED_BACKEND="libusb"
  fi

  echo "Backend for ${patch_name}: reported=${REPORTED_BACKEND}, pinned=${PINNED_BACKEND}"

  if [[ "${HAS_LIBUSB}" != "${TARGET_LIBUSB}" ]]; then
    echo "Error: Backend mismatch for ${patch_name}! Reported ${REPORTED_BACKEND} but pinned ${PINNED_BACKEND}" >&2
    exit 1
  fi

  # Run compat test suite against the mutant binary with the pinned backend
  if ADB_CLIENT="${BUILT_ADB}" ADB_SERVER_BIN="${BUILT_ADB}" ADB_REFERENCE="${BUILT_ADB}" ADB_LIBUSB="${TARGET_LIBUSB}" ${TEST_RUNNER} > /dev/null 2>&1; then
    echo "Result for ${patch_name}: SURVIVED (backend=${REPORTED_BACKEND})"
    echo "${patch_name},survived" >> "${RESULTS_FILE}"
  else
    echo "Result for ${patch_name}: KILLED (backend=${REPORTED_BACKEND})"
    echo "${patch_name},killed" >> "${RESULTS_FILE}"
  fi
done

# Clean up build tree after mutation run
"${REPO_ROOT}/scripts/build-cpp-adb.sh" > /dev/null 2>&1 || true

echo "=========================================="
echo "Mutation testing finished. Results saved to ${RESULTS_FILE}:"
cat "${RESULTS_FILE}"
