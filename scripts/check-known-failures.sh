#!/usr/bin/env bash
set -euo pipefail

# check-known-failures.sh
# Checks known-failures lists in tests/compat/:
# - tests/compat/rust-client-known-failures.txt (using ADB_CLIENT as set or default)
# - tests/compat/rust-server-known-failures.txt (using ADB_SERVER_BIN set to Rust adb)
#
# 1. Parses test names and comments.
# 2. Ensures every listed test exists in cargo test list.
# 3. Runs each listed test with --exact.
# 4. Outputs markdown table of results to target/known_failures_report.md and stdout.
# 5. Fails if any listed test passes or if a test name matches no test.

CLIENT_FAILURES_FILE="tests/compat/rust-client-known-failures.txt"
SERVER_FAILURES_FILE="tests/compat/rust-server-known-failures.txt"
REPORT_FILE="target/known_failures_report.md"

mkdir -p target

echo "=== Getting list of available tests in compat crate ==="
if cargo nextest --version >/dev/null 2>&1; then
AVAILABLE_TESTS=$(cargo nextest list -p compat --message-format json 2>/dev/null | python3 -c '
import json, sys
data = json.load(sys.stdin)
for binary in data.get("rust-suites", {}).values():
    for testcase_name in binary.get("testcases", {}).keys():
        print(testcase_name)
')
else
AVAILABLE_TESTS=$(cargo test -p compat -- --list 2>/dev/null | grep ": test" | sed -e 's/: test$//' -e 's/^[[:space:]]*//')
fi

check_failures_file() {
    local failures_file="$1"
    local mode_name="$2"
    local env_server_bin="${3:-}"

    if [ ! -f "$failures_file" ]; then
        return 0
    fi

    echo "=== Checking $mode_name known failures: $failures_file ==="

    declare -a TEST_NAMES=()
    declare -a TEST_REASONS=()
    local CURRENT_REASON=""

    while IFS= read -r line || [ -n "$line" ]; do
        local trimmed
        trimmed=$(echo "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')

        if [ -z "$trimmed" ]; then
            CURRENT_REASON=""
        elif [[ "$trimmed" =~ ^# ]]; then
            local comment_text
            comment_text=$(echo "$trimmed" | sed 's/^#[[:space:]]*//')
            if [ -n "$CURRENT_REASON" ]; then
                CURRENT_REASON="$CURRENT_REASON; $comment_text"
            else
                CURRENT_REASON="$comment_text"
            fi
        else
            TEST_NAMES+=("$trimmed")
            if [ -n "$CURRENT_REASON" ]; then
                TEST_REASONS+=("$CURRENT_REASON")
            else
                TEST_REASONS+=("No reason provided")
            fi
            CURRENT_REASON=""
        fi
    done < "$failures_file"

    if [ ${#TEST_NAMES[@]} -eq 0 ]; then
        echo "No known failures listed in $failures_file."
        return 0
    fi

    # Step 1: Validate all listed test names exist in available tests
    local MISSING_TESTS=()
    for test_name in "${TEST_NAMES[@]}"; do
        if ! echo "$AVAILABLE_TESTS" | grep -F -x -q "$test_name"; then
            MISSING_TESTS+=("$test_name")
        fi
    done

    if [ ${#MISSING_TESTS[@]} -ne 0 ]; then
        echo "Error: The following tests listed in $failures_file do not exist in cargo test list -p compat:" >&2
        for missing in "${MISSING_TESTS[@]}"; do
            echo "  - $missing" >&2
        done
        exit 1
    fi

    # Step 2: Run each test on its own with --exact and record pass/fail
    declare -a TEST_RESULTS=()
    local UNEXPECTED_PASSES=()

    echo "=== Running ${#TEST_NAMES[@]} $mode_name known failing test(s) individually with --exact ==="

    for i in "${!TEST_NAMES[@]}"; do
        local test_name="${TEST_NAMES[$i]}"
        local reason="${TEST_REASONS[$i]}"

        echo "Running test [$((i+1))/${#TEST_NAMES[@]}]: $test_name ..."

        local test_passed=0
        if [ -n "$env_server_bin" ]; then
            if cargo nextest --version >/dev/null 2>&1; then
                if ADB_SERVER_BIN="$env_server_bin" cargo nextest run -p compat -- "$test_name" --exact > /dev/null 2>&1; then
                    test_passed=1
                fi
            else
                if ADB_SERVER_BIN="$env_server_bin" cargo test -p compat -- "$test_name" --exact > /dev/null 2>&1; then
                    test_passed=1
                fi
            fi
        else
            if cargo nextest --version >/dev/null 2>&1; then
                if cargo nextest run -p compat -- "$test_name" --exact > /dev/null 2>&1; then
                    test_passed=1
                fi
            else
                if cargo test -p compat -- "$test_name" --exact > /dev/null 2>&1; then
                    test_passed=1
                fi
            fi
        fi

        if [ $test_passed -eq 1 ]; then
            TEST_RESULTS+=("PASS (Unexpected)")
            UNEXPECTED_PASSES+=("$test_name")
        else
            TEST_RESULTS+=("FAIL (Expected)")
        fi
    done

    # Output to markdown report
    {
        echo "### $mode_name Known Failures"
        echo ""
        echo "| Test Name | Status | Reason |"
        echo "| --- | --- | --- |"
        for i in "${!TEST_NAMES[@]}"; do
            echo "| \`${TEST_NAMES[$i]}\` | ${TEST_RESULTS[$i]} | ${TEST_REASONS[$i]} |"
        done
        echo ""
    } >> "$REPORT_FILE"

    if [ ${#UNEXPECTED_PASSES[@]} -ne 0 ]; then
        echo "Error: The following tests in $failures_file PASSED when expected to fail:" >&2
        for passed in "${UNEXPECTED_PASSES[@]}"; do
            echo "  - $passed" >&2
        done
        echo "Please remove passed tests from $failures_file." >&2
        exit 1
    fi

    echo "All ${#TEST_NAMES[@]} listed $mode_name known-failing tests failed as expected."
}

# Reset report file
echo "## Known Failures Enforcement Report" > "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

# 1. Check client known failures
check_failures_file "$CLIENT_FAILURES_FILE" "Client" ""

# 2. Check server known failures with Rust server if present or built
RUST_SERVER_BIN="$(pwd)/target/debug/adb"
if [ ! -x "$RUST_SERVER_BIN" ] && [ -x "$(pwd)/target/debug/adb.exe" ]; then
    RUST_SERVER_BIN="$(pwd)/target/debug/adb.exe"
fi

if [ -f "$SERVER_FAILURES_FILE" ]; then
    if [ ! -x "$RUST_SERVER_BIN" ]; then
        echo "Building adb-cli for server known failures check..."
        cargo build -p adb-cli
    fi
    check_failures_file "$SERVER_FAILURES_FILE" "Server" "$RUST_SERVER_BIN"
fi

echo ""
echo "=== Known Failures Enforcement Report ==="
cat "$REPORT_FILE"
echo ""
