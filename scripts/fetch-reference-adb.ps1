# Fetch reference ADB binary for Windows (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2), checked in under original/.
#
# Reference ADB binaries matching this revision are included in binaries/ in the repository
# and populated into target/reference/.

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path "$ScriptDir\.."
$TargetDir = Join-Path $RepoRoot "target\reference"
$AdbBin = Join-Path $TargetDir "adb.exe"

if (Test-Path $AdbBin) {
    Write-Output $AdbBin
    exit 0
}

$SrcDir = Join-Path $RepoRoot "binaries\win"
if (-not (Test-Path $SrcDir)) {
    $SrcDir = Join-Path $RepoRoot "binaries\windows"
}

if (-not (Test-Path $SrcDir)) {
    [Console]::Error.WriteLine("Reference binary directory not found at $SrcDir")
    exit 1
}

New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null
Copy-Item -Path "$SrcDir\*" -Destination $TargetDir -Recurse -Force

Write-Output $AdbBin
