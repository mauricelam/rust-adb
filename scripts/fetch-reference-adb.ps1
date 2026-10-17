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
    $ZipPath = Join-Path $TargetDir "platform-tools.zip"
    $TempExtract = Join-Path $TargetDir "temp_extract"

    [Console]::Error.WriteLine("Downloading reference ADB from $Url...")
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath | Out-Null

    Expand-Archive -Path $ZipPath -DestinationPath $TempExtract -Force | Out-Null

    $ExtractedItems = Get-ChildItem -Path (Join-Path $TempExtract "platform-tools")
    foreach ($item in $ExtractedItems) {
        Move-Item -Path $item.FullName -Destination $TargetDir -Force | Out-Null
    }

    Remove-Item -Recurse -Force $TempExtract | Out-Null
    Remove-Item -Force $ZipPath | Out-Null
}

Write-Output $AdbBin
