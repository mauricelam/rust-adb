# Fetch reference ADB binary for Windows (version 36.0.2 / build 14143358).
# Pinned upstream ADB revision: platform-tools 36.0.2 (Build 14143358) from
# platform/packages/modules/adb (android15-tests-dev / platform-tools-36.0.2), checked in under original/.
#
# Reference ADB binaries are copied from binaries/ in the repository if present,
# or downloaded from official platform-tools releases into target/reference/.

$ErrorActionPreference = "Stop"

# Ensure 32-bit VC++ Redistributable runtime is installed on 64-bit Windows Server VMs
if (-not (Test-Path "$env:SystemRoot\SysWOW64\vcruntime140.dll")) {
    try {
        [Console]::Error.WriteLine("Installing 32-bit Visual C++ Redistributable runtime...")
        $VcUrl = "https://aka.ms/vs/17/release/vc_redist.x86.exe"
        $VcPath = Join-Path $env:TEMP "vc_redist.x86.exe"
        Invoke-WebRequest -Uri $VcUrl -OutFile $VcPath -UseBasicParsing
        Start-Process -FilePath $VcPath -ArgumentList "/quiet", "/norestart" -Wait
        Remove-Item -Force $VcPath -ErrorAction SilentlyContinue
    } catch {
        [Console]::Error.WriteLine("Warning: Failed to install vc_redist.x86.exe: $_")
    }
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path "$ScriptDir\..").Path
$TargetDir = Join-Path $RepoRoot "target\reference"
$AdbBin = Join-Path $TargetDir "adb.exe"
$AdbDll = Join-Path $TargetDir "AdbWinApi.dll"

if ((Test-Path $AdbBin) -and (Test-Path $AdbDll)) {
    Write-Output $AdbBin
    exit 0
}

$SrcDir = Join-Path $RepoRoot "binaries\win"
if (-not (Test-Path $SrcDir)) {
    $SrcDir = Join-Path $RepoRoot "binaries\windows"
}

New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null

if (Test-Path $SrcDir) {
    Get-ChildItem -Path $SrcDir | Copy-Item -Destination $TargetDir -Recurse -Force | Out-Null
} else {
    $Url = "https://dl.google.com/android/repository/platform-tools_r36.0.2-win.zip"
    $ZipPath = Join-Path $RepoRoot "temp_reference.zip"
    $TempExtract = Join-Path $RepoRoot "temp_reference_extract"

    [Console]::Error.WriteLine("Downloading reference ADB from $Url...")
    $null = Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing

    Expand-Archive -Path $ZipPath -DestinationPath $TempExtract -Force | Out-Null

    $ExtractedDir = Join-Path $TempExtract "platform-tools"
    if (Test-Path $ExtractedDir) {
        Get-ChildItem -Path $ExtractedDir | Copy-Item -Destination $TargetDir -Recurse -Force | Out-Null
    }

    Remove-Item -Recurse -Force $TempExtract -ErrorAction SilentlyContinue | Out-Null
    Remove-Item -Force $ZipPath -ErrorAction SilentlyContinue | Out-Null
}

if (Get-Command Unblock-File -ErrorAction SilentlyContinue) {
    Get-ChildItem -Path $TargetDir -Recurse -ErrorAction SilentlyContinue | Unblock-File -ErrorAction SilentlyContinue
}

Write-Output $AdbBin
