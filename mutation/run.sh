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

for patch_file in "${PATCH_FILES[@]}"; do
  patch_name="$(basename "${patch_file}")"
  echo "=========================================="
  echo "Running mutation test for: ${patch_name}"
  echo "=========================================="

  # Build adb with the patch applied
  BUILT_ADB="$("${REPO_ROOT}/scripts/build-cpp-adb.sh" --patch "${patch_file}")"

  # Run compat test suite against the mutant binary
  if ADB_CLIENT="${BUILT_ADB}" ADB_SERVER_BIN="${BUILT_ADB}" ADB_REFERENCE="${BUILT_ADB}" cargo test -p compat -- --test-threads=1 > /dev/null 2>&1; then
    echo "Result for ${patch_name}: SURVIVED"
    echo "${patch_name},survived" >> "${RESULTS_FILE}"
  else
    echo "Result for ${patch_name}: KILLED"
    echo "${patch_name},killed" >> "${RESULTS_FILE}"
  fi
done

# Clean up build tree after mutation run
"${REPO_ROOT}/scripts/build-cpp-adb.sh" > /dev/null 2>&1 || true

echo "=========================================="
echo "Mutation testing finished. Results saved to ${RESULTS_FILE}:"
cat "${RESULTS_FILE}"
