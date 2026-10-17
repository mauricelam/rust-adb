# Fetches the reference ADB C++ binary matching pinned upstream revision platform-tools-36.0.2 (Build 14143358).
# Uses official platform-tools r36.0.2 binaries since full C++ source build is deferred to task 0.10.
# Idempotent: safe to run multiple times.

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RepoRoot = Resolve-Path "$ScriptDir\.."
$TargetDir = Join-Path $RepoRoot "target\reference"
$AdbBin = Join-Path $TargetDir "platform-tools\adb.exe"

if (Test-Path $AdbBin) {
    Write-Output $AdbBin
    exit 0
}

if (-not (Test-Path $TargetDir)) {
    New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null
}

$Url = "https://dl.google.com/android/repository/platform-tools_r36.0.2-win.zip"
$ZipFile = Join-Path $TargetDir "platform-tools.zip"

Invoke-WebRequest -Uri $Url -OutFile $ZipFile
Expand-Archive -Path $ZipFile -DestinationPath $TargetDir -Force
Remove-Item -Path $ZipFile -Force

Write-Output $AdbBin
