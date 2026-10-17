# Fetch reference ADB binary for Windows (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2), checked in under original/.
#
# Reference ADB binaries are copied from binaries/ in the repository if present,
# or downloaded from official platform-tools releases into target/reference/.

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path "$ScriptDir\..").Path
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

New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null

if (Test-Path $SrcDir) {
    Copy-Item -Path "$SrcDir\*" -Destination $TargetDir -Recurse -Force | Out-Null
} else {
    $Url = "https://dl.google.com/android/repository/platform-tools_r36.0.2-win.zip"
    $ZipPath = Join-Path $RepoRoot "temp_reference.zip"
    $TempExtract = Join-Path $RepoRoot "temp_reference_extract"

    [Console]::Error.WriteLine("Downloading reference ADB from $Url...")
    $null = Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing

    Expand-Archive -Path $ZipPath -DestinationPath $TempExtract -Force | Out-Null

    $ExtractedDir = Join-Path $TempExtract "platform-tools"
    if (Test-Path $ExtractedDir) {
        Copy-Item -Path "$ExtractedDir\*" -Destination $TargetDir -Recurse -Force | Out-Null
    }

    Remove-Item -Recurse -Force $TempExtract -ErrorAction SilentlyContinue | Out-Null
    Remove-Item -Force $ZipPath -ErrorAction SilentlyContinue | Out-Null
}

if (Get-Command Unblock-File -ErrorAction SilentlyContinue) {
    Get-ChildItem -Path $TargetDir -Recurse -ErrorAction SilentlyContinue | Unblock-File -ErrorAction SilentlyContinue
}

Write-Output $AdbBin
