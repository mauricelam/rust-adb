#!/usr/bin/env bash
#
# gen-parity.sh - Generates and updates docs/parity.md tracking parity
# for ADB subcommands, host services, and local services.
#
# How status preservation / merging works:
# When run, this script parses subcommands from '$ADB_REFERENCE help' and services
# from 'original/docs/dev/services.md' (SERVICES.TXT).
# If 'docs/parity.md' already exists, the script reads existing rows and extracts
# their 'Test Status', 'Rust Status', and 'Notes' columns keyed by (Name, Kind).
# For any row already present in 'docs/parity.md', its existing statuses and notes are preserved.
# For any new row discovered, default statuses ('none', 'not-started') and default notes are assigned.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

python3 - << 'EOF'
import os
import sys
import subprocess
import re

REPO_ROOT = os.environ.get("REPO_ROOT", os.getcwd())
PARITY_FILE = os.path.join(REPO_ROOT, "docs", "parity.md")
SERVICES_FILE = os.path.join(REPO_ROOT, "original", "docs", "dev", "services.md")

def get_adb_bin():
    if "ADB_REFERENCE" in os.environ and os.path.exists(os.environ["ADB_REFERENCE"]):
        return os.environ["ADB_REFERENCE"]
    for fallback in [
        os.path.join(REPO_ROOT, "binaries", "linux", "adb"),
        os.path.join(REPO_ROOT, "binaries", "mac", "adb"),
        os.path.join(REPO_ROOT, "binaries", "windows", "adb.exe"),
    ]:
        if os.path.exists(fallback):
            return fallback
    return "adb"

def read_existing_parity(parity_path):
    existing = {}
    if not os.path.exists(parity_path):
        return existing
    with open(parity_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line.startswith("|") or "Test Status" in line or "---" in line:
                continue
            parts = [p.strip() for p in line.split("|")[1:-1]]
            if len(parts) >= 5:
                raw_name, kind, test_status, rust_status, notes = parts[0], parts[1], parts[2], parts[3], parts[4]
                clean_name = raw_name.strip("`")
                existing[(clean_name, kind)] = (test_status, rust_status, notes)
    return existing

def extract_commands(adb_bin):
    res = subprocess.run([adb_bin, "help"], capture_output=True, text=True)
    if res.returncode != 0:
        sys.exit(f"Error running adb help: {res.stderr}")
    lines = res.stdout.splitlines()

    commands = []
    current_section = None

    for line in lines:
        if not line:
            continue
        if not line.startswith(" ") and line.endswith(":"):
            current_section = line[:-1]
            continue
        if current_section == "environment variables":
            continue

        if line.startswith(" -") or line.startswith(" --"):
            parts = line.strip().split()
            cmd = parts[0]
            commands.append(cmd)
        elif line.startswith(" ") and not line.startswith("     "):
            parts = line.strip().split()
            if not parts:
                continue
            cmd = parts[0]
            if cmd == "mdns" and len(parts) > 1 and parts[1] in ("check", "services"):
                cmd = f"mdns {parts[1]}"
            elif cmd in ("forward", "reverse") and len(parts) > 1 and parts[1].startswith("--"):
                cmd = f"{cmd} {parts[1]}"
            elif cmd == "reconnect" and len(parts) > 1 and parts[1] in ("device", "offline"):
                cmd = f"{cmd} {parts[1]}"
            elif cmd.startswith("wait-for"):
                cmd = line.strip().split()[0]
            commands.append(cmd)

    seen = set()
    unique_commands = []
    for c in commands:
        if c not in seen:
            seen.add(c)
            unique_commands.append(c)
    return unique_commands

def extract_services(services_file):
    if not os.path.exists(services_file):
        sys.exit(f"Services file not found: {services_file}")
    with open(services_file, "r", encoding="utf-8") as f:
        content = f.read()

    host_sec = content.split("HOST SERVICES:")[1].split("LOCAL SERVICES:")[0]
    local_sec = content.split("LOCAL SERVICES:")[1]

    def parse_sec(text):
        res = []
        for line in text.splitlines():
            if line and not line[0].isspace():
                if line.startswith("This file") or line.startswith("All the queries") or line.startswith("to a real") or line.startswith("above.") or line.startswith("```"):
                    continue
                note = ""
                name = line.strip()
                if " (API>=" in name:
                    parts = name.split(" (API>=")
                    name = parts[0]
                    note = "API>=" + parts[1].rstrip(")")
                res.append((name, note))
        return res

    host_services = parse_sec(host_sec)
    local_services = parse_sec(local_sec)
    return host_services, local_services

def main():
    adb_bin = get_adb_bin()
    existing = read_existing_parity(PARITY_FILE)
    commands = extract_commands(adb_bin)
    host_services, local_services = extract_services(SERVICES_FILE)

    os.makedirs(os.path.dirname(PARITY_FILE), exist_ok=True)

    out = []
    out.append("# ADB Parity Tracker\n")
    out.append("This document tracks parity for all ADB subcommands, smart-socket host services, and local services.\n")
    out.append("It is generated by `scripts/gen-parity.sh`. When re-run, existing status and notes columns are preserved by merging.\n")

    out.append("## Subcommands and Global Options\n")
    out.append("| Name | Kind | Test Status | Rust Status | Notes |")
    out.append("| --- | --- | --- | --- | --- |")

    for cmd in commands:
        kind = "command"
        key = (cmd, kind)
        if key in existing:
            t_stat, r_stat, notes = existing[key]
        else:
            t_stat, r_stat, notes = "none", "not-started", ""
        out.append(f"| `{cmd}` | {kind} | {t_stat} | {r_stat} | {notes} |")

    out.append("\n## Host Services\n")
    out.append("| Name | Kind | Test Status | Rust Status | Notes |")
    out.append("| --- | --- | --- | --- | --- |")

    for name, init_note in host_services:
        kind = "host service"
        key = (name, kind)
        if key in existing:
            t_stat, r_stat, notes = existing[key]
        else:
            t_stat, r_stat, notes = "none", "not-started", init_note
        out.append(f"| `{name}` | {kind} | {t_stat} | {r_stat} | {notes} |")

    out.append("\n## Local Services\n")
    out.append("| Name | Kind | Test Status | Rust Status | Notes |")
    out.append("| --- | --- | --- | --- | --- |")

    for name, init_note in local_services:
        kind = "local service"
        key = (name, kind)
        if key in existing:
            t_stat, r_stat, notes = existing[key]
        else:
            t_stat, r_stat, notes = "none", "not-started", init_note
        out.append(f"| `{name}` | {kind} | {t_stat} | {r_stat} | {notes} |")

    out.append("")

    with open(PARITY_FILE, "w", encoding="utf-8") as f:
        f.write("\n".join(out))

    print(f"Successfully generated {PARITY_FILE}")

if __name__ == "__main__":
    main()
EOF
