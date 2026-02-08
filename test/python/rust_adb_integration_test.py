import subprocess
import pytest
import os
import re
import time

# Configuration from environment variables
ADB_BINARY = os.environ.get('ADB_BINARY', 'adb')
RUST_ADB_BINARY = os.environ.get('RUST_ADB_BINARY', 'rust-adb')
ADB_PORT = os.environ.get('ADB_SERVER_PORT', '5037')

@pytest.fixture(scope="module", autouse=True)
def adb_server():
    """
    Phase 1: Start Server
    Ensure the official C++ adb server is initialized and listening.
    """
    print(f"\nStarting ADB server using {ADB_BINARY} on port {ADB_PORT}...")
    try:
        # Starting the server by running 'devices'
        # -P specifies the port
        subprocess.run([ADB_BINARY, "-P", ADB_PORT, "devices"], check=True, capture_output=True)
        # Give it a moment to stabilize
        time.sleep(1)
        yield
    finally:
        # Teardown: Kill the adb server
        print(f"\nKilling ADB server on port {ADB_PORT}...")
        subprocess.run([ADB_BINARY, "-P", ADB_PORT, "kill-server"], check=False)

def run_rust_adb(args):
    """
    Phase 2: Rust Execution
    Run the Rust Port binary configured to connect to the existing server.
    """
    cmd = [RUST_ADB_BINARY, "-P", ADB_PORT] + args
    print(f"Running Rust ADB: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    return result

def test_rust_adb_version():
    """
    Test Case: version
    Ensure the Rust client reports compatible versioning.
    """
    result = run_rust_adb(["version"])
    assert result.returncode == 0, f"Rust ADB version failed: {result.stderr}"

    # Expected regex for version (matching official ADB style)
    assert re.search(r"Android Debug Bridge version", result.stdout), \
        f"Output did not match version pattern: {result.stdout}"

def test_rust_adb_devices():
    """
    Test Case: devices
    Check if the Rust port lists the same serial numbers as the official client.
    """
    # Get official output for comparison
    official_result = subprocess.run([ADB_BINARY, "-P", ADB_PORT, "devices"],
                                     capture_output=True, text=True, check=True)

    # Get rust output
    rust_result = run_rust_adb(["devices"])

    assert rust_result.returncode == 0, f"Rust ADB devices failed: {rust_result.stderr}"

    def extract_serials(output):
        lines = output.strip().splitlines()
        serials = set()
        for line in lines:
            if line.startswith("List of devices attached") or not line.strip():
                continue
            if '\t' in line:
                serials.add(line.split('\t')[0])
        return serials

    official_serials = extract_serials(official_result.stdout)
    rust_serials = extract_serials(rust_result.stdout)

    assert rust_serials == official_serials, \
        f"Serial numbers mismatch. Official: {official_serials}, Rust: {rust_serials}"

def test_rust_adb_shell_getprop():
    """
    Test Case: shell getprop ro.product.model
    Verify string output from a connected device.
    """
    # Check if there are any devices first
    official_devices = subprocess.run([ADB_BINARY, "-P", ADB_PORT, "devices"],
                                      capture_output=True, text=True, check=True)

    lines = official_devices.stdout.strip().splitlines()
    if len(lines) <= 1:
        pytest.skip("No device connected to test shell command")

    # Find the first connected device serial
    serial = None
    for line in lines[1:]:
        if '\t' in line and 'device' in line:
            serial = line.split('\t')[0]
            break

    if not serial:
        pytest.skip("No online device found")

    # Run command via Rust ADB
    rust_result = run_rust_adb(["-s", serial, "shell", "getprop", "ro.product.model"])

    assert rust_result.returncode == 0, f"Rust ADB shell failed: {rust_result.stderr}"
    assert len(rust_result.stdout.strip()) > 0, "Prop output was empty"

    # Optionally compare with official output
    official_prop = subprocess.run([ADB_BINARY, "-P", ADB_PORT, "-s", serial, "shell", "getprop", "ro.product.model"],
                                   capture_output=True, text=True, check=True)

    assert rust_result.stdout.strip() == official_prop.stdout.strip(), \
        f"Prop output mismatch. Official: {official_prop.stdout.strip()}, Rust: {rust_result.stdout.strip()}"

def test_rust_adb_integration():
    """
    Master integration test that covers multiple commands in one go,
    as requested in the example structure.
    """
    # 1. Version
    assert run_rust_adb(["version"]).returncode == 0

    # 2. Devices
    assert run_rust_adb(["devices"]).returncode == 0

    # 3. Shell (if device available)
    official_devices = subprocess.run([ADB_BINARY, "-P", ADB_PORT, "devices"],
                                      capture_output=True, text=True, check=True)
    lines = official_devices.stdout.strip().splitlines()
    if len(lines) > 1:
        # Just check if shell command returns success
        assert run_rust_adb(["shell", "echo", "hello"]).returncode == 0
