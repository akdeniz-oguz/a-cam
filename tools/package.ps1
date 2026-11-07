<#
.SYNOPSIS
    Collects a-Cam into a distributable folder.

.DESCRIPTION
    Builds the application and copies everything it needs into dist\a-cam:
    the exe, the MinGW runtime DLLs, the virtual camera source and the scrcpy tools.

    The MinGW DLLs are required because the H.264 decoder (openh264) is written in C++ and
    the exe links against libstdc++. Static linking was tried; the linker prefers the import
    library next to the static archive, so it did not work.

.EXAMPLE
    .\package.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$release = Join-Path $root 'target\release'
$dist = Join-Path $root 'dist\a-cam'

$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Filter 'gcc.exe' -Recurse `
    -Depth 6 -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $mingw) {
    Write-Host "MinGW not found. Install it with: winget install BrechtSanders.WinLibs.POSIX.MSVCRT" -ForegroundColor Red
    exit 1
}
$mingwBin = $mingw.DirectoryName
$env:PATH = "$mingwBin;$env:PATH"

Write-Host "Building..."
& cargo build --release
if ($LASTEXITCODE -ne 0) { exit 1 }

if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Path $dist | Out-Null

Copy-Item (Join-Path $release 'a-cam.exe') $dist
Copy-Item (Join-Path $release 'a_cam_source.dll') $dist
foreach ($dll in @('libstdc++-6.dll', 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll')) {
    Copy-Item (Join-Path $mingwBin $dll) $dist
}

$scrcpy = Join-Path $root 'scrcpy'
if (Test-Path $scrcpy) {
    Copy-Item $scrcpy (Join-Path $dist 'scrcpy') -Recurse
} else {
    Write-Host "WARNING: no scrcpy folder, so it was not included." -ForegroundColor Yellow
}
Copy-Item (Join-Path $PSScriptRoot 'install-vcam.ps1') $dist

$size = [math]::Round((Get-ChildItem $dist -Recurse | Measure-Object Length -Sum).Sum / 1MB, 1)
Write-Host "Ready: $dist  ($size MB)" -ForegroundColor Green
Write-Host "On the target machine, run once as administrator to enable the virtual camera:"
Write-Host "  .\install-vcam.ps1"
