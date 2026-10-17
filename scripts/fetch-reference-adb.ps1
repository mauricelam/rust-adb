# Fetch reference ADB binary for Windows (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2).
#
# Note: Building full C++ ADB from source requires the full Android source tree / Soong build system.
# Therefore, official platform-tools binaries corresponding to the pinned revision are fetched.

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path "$ScriptDir\.."
$TargetDir = Join-Path $RepoRoot "target\reference"
$AdbBin = Join-Path $TargetDir "adb.exe"

if (Test-Path $AdbBin) {
    Write-Output $AdbBin
    exit 0
}

New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null

$Url = "https://dl.google.com/android/repository/platform-tools_r36.0.2-win.zip"
$ZipPath = Join-Path $TargetDir "platform-tools.zip"
$TempExtract = Join-Path $TargetDir "temp_extract"

[Console]::Error.WriteLine("Downloading reference ADB from $Url...")
Invoke-WebRequest -Uri $Url -OutFile $ZipPath

Expand-Archive -Path $ZipPath -DestinationPath $TempExtract -Force

$ExtractedItems = Get-ChildItem -Path (Join-Path $TempExtract "platform-tools")
foreach ($item in $ExtractedItems) {
    Move-Item -Path $item.FullName -Destination $TargetDir -Force
}

Remove-Item -Recurse -Force $TempExtract
Remove-Item -Force $ZipPath

Write-Output $AdbBin
