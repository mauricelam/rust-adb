# Fetch / prepare reference C++ adb binary matching pinned platform-tools release (36.0.2 / Build 14143358).
# Copies prebuilt reference adb binary to target/reference/platform-tools/ for Windows.

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path "$ScriptDir\.." | Select-Object -ExpandProperty Path

$DestDir = Join-Path $RepoRoot "target\reference\platform-tools"
if (-not (Test-Path $DestDir)) {
    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
}

$SrcDir = Join-Path $RepoRoot "binaries\windows"
if (-not (Test-Path $SrcDir)) {
    Write-Error "Prebuilt binaries directory $SrcDir does not exist."
    exit 1
}

Copy-Item -Path "$SrcDir\*" -Destination $DestDir -Recurse -Force

Write-Output (Join-Path $DestDir "adb.exe")
