#!/usr/bin/env bash
set -euo pipefail

# check-known-failures.sh
# Checks known-failures list in tests/compat/rust-client-known-failures.txt:
# 1. Parses test names and comments.
# 2. Ensures every listed test exists in cargo nextest list -p compat.
# 3. Runs each listed test with --exact.
# 4. Outputs markdown table of results to target/known_failures_report.md and stdout.
# 5. Fails if any listed test passes or if a test name matches no test.

KNOWN_FAILURES_FILE="tests/compat/rust-client-known-failures.txt"
REPORT_FILE="target/known_failures_report.md"

mkdir -p target

if [ ! -f "$KNOWN_FAILURES_FILE" ]; then
    echo "Error: Known failures file $KNOWN_FAILURES_FILE not found." >&2
    exit 1
fi

echo "=== Getting list of available tests in compat crate ==="
AVAILABLE_TESTS=$(cargo nextest list -p compat --message-format json 2>/dev/null | python3 -c '
import json, sys
data = json.load(sys.stdin)
for binary in data.get("rust-suites", {}).values():
    for testcase_name in binary.get("testcases", {}).keys():
        print(testcase_name)
')

# Collect tests and their reasons
# Reason is the comment line(s) directly preceding the test line.
declare -a TEST_NAMES=()
declare -a TEST_REASONS=()

CURRENT_REASON=""

while IFS= read -r line || [ -n "$line" ]; do
    # Trim leading/trailing whitespace
    trimmed=$(echo "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')

    if [ -z "$trimmed" ]; then
        # Empty line resets accumulated reason
        CURRENT_REASON=""
    elif [[ "$trimmed" =~ ^# ]]; then
        # Comment line
        comment_text=$(echo "$trimmed" | sed 's/^#[[:space:]]*//')
        if [ -n "$CURRENT_REASON" ]; then
            CURRENT_REASON="$CURRENT_REASON; $comment_text"
        else
            CURRENT_REASON="$comment_text"
        fi
    else
        # Test entry
        TEST_NAMES+=("$trimmed")
        if [ -n "$CURRENT_REASON" ]; then
            TEST_REASONS+=("$CURRENT_REASON")
        else
            TEST_REASONS+=("No reason provided")
        fi
        CURRENT_REASON=""
    fi
done < "$KNOWN_FAILURES_FILE"

if [ ${#TEST_NAMES[@]} -eq 0 ]; then
    echo "No known failures listed in $KNOWN_FAILURES_FILE."
    exit 0
fi

# Step 1: Validate all listed test names exist in available tests
MISSING_TESTS=()
for test_name in "${TEST_NAMES[@]}"; do
    if ! echo "$AVAILABLE_TESTS" | grep -F -x -q "$test_name"; then
        MISSING_TESTS+=("$test_name")
    fi
done

if [ ${#MISSING_TESTS[@]} -ne 0 ]; then
    echo "Error: The following tests listed in $KNOWN_FAILURES_FILE do not exist in cargo nextest list -p compat:" >&2
    for missing in "${MISSING_TESTS[@]}"; do
        echo "  - $missing" >&2
    done
    exit 1
fi

# Step 2: Run each test on its own with --exact and record pass/fail
declare -a TEST_RESULTS=()
UNEXPECTED_PASSES=()

echo "=== Running ${#TEST_NAMES[@]} known failing test(s) individually with --exact ==="

for i in "${!TEST_NAMES[@]}"; do
    test_name="${TEST_NAMES[$i]}"
    reason="${TEST_REASONS[$i]}"

    echo "Running test [$((i+1))/${#TEST_NAMES[@]}]: $test_name ..."

    # Run test
    if cargo nextest run -p compat -- "$test_name" --exact > /dev/null 2>&1; then
        TEST_RESULTS+=("PASS (Unexpected)")
        UNEXPECTED_PASSES+=("$test_name")
    else
        TEST_RESULTS+=("FAIL (Expected)")
    fi
done

# Step 3: Format markdown table and report
{
    echo "| Test Name | Status | Reason |"
    echo "| --- | --- | --- |"
    for i in "${!TEST_NAMES[@]}"; do
        echo "| \`${TEST_NAMES[$i]}\` | ${TEST_RESULTS[$i]} | ${TEST_REASONS[$i]} |"
    done
} > "$REPORT_FILE"

echo ""
echo "=== Known Failures Enforcement Report ==="
cat "$REPORT_FILE"
echo ""

if [ ${#UNEXPECTED_PASSES[@]} -ne 0 ]; then
    echo "Error: The following tests in $KNOWN_FAILURES_FILE PASSED when expected to fail:" >&2
    for passed in "${UNEXPECTED_PASSES[@]}"; do
        echo "  - $passed" >&2
    done
    echo "Please remove passed tests from $KNOWN_FAILURES_FILE." >&2
    exit 1
fi

echo "All ${#TEST_NAMES[@]} listed known-failing tests failed as expected."
