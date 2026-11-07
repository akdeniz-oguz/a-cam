<#
.SYNOPSIS
    Installs or removes the a-Cam virtual camera source (a_cam_source.dll).

.DESCRIPTION
    The DLL is loaded by the Windows "Frame Server" (Local Service) and "Frame Server
    Monitor" (Local System) services. Those services cannot read folders under a user
    profile, so leaving the DLL in target\release makes starting the camera fail with an
    access error. The DLL is therefore copied to Program Files and registered from there.

    Registration writes to HKEY_LOCAL_MACHINE, which needs administrator rights.

.EXAMPLE
    .\install-vcam.ps1
    .\install-vcam.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin = ([Security.Principal.WindowsPrincipal]$identity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "This script must run as administrator." -ForegroundColor Red
    Write-Host "Open PowerShell with 'Run as administrator' and try again."
    exit 1
}

$installDir = Join-Path $env:ProgramFiles 'a-cam'
$installedDll = Join-Path $installDir 'a_cam_source.dll'

# regsvr32 is a GUI program. Launched with '&', PowerShell does not wait for it: the exit
# code comes back empty and the file stays locked for a moment, so it cannot be replaced.
# Start-Process -Wait both waits for it and returns the real exit code.
function Invoke-RegSvr32 {
    param([string]$Path, [switch]$Unregister)
    $arguments = @('/s')
    if ($Unregister) { $arguments += '/u' }
    $arguments += "`"$Path`""
    $process = Start-Process -FilePath regsvr32.exe -ArgumentList $arguments -Wait -PassThru
    return $process.ExitCode
}

if ($Uninstall) {
    if (Test-Path $installedDll) {
        [void](Invoke-RegSvr32 -Path $installedDll -Unregister)
        Write-Host "Unregistered." -ForegroundColor Green
        Remove-Item $installedDll -Force -ErrorAction SilentlyContinue
    }
    if ((Test-Path $installDir) -and -not (Get-ChildItem $installDir)) {
        Remove-Item $installDir -Force
    }
    Write-Host "Removal complete." -ForegroundColor Green
    exit 0
}

$builtDll = Join-Path $PSScriptRoot '..\target\release\a_cam_source.dll'
if (-not (Test-Path $builtDll)) {
    Write-Host "a_cam_source.dll not found at: $builtDll" -ForegroundColor Red
    Write-Host "Build it first:  cargo build --release"
    exit 1
}

if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir | Out-Null
}

# The installed DLL may be in use, so unregister before overwriting it.
if (Test-Path $installedDll) {
    [void](Invoke-RegSvr32 -Path $installedDll -Unregister)
}
try {
    Copy-Item $builtDll $installedDll -Force
} catch [System.IO.IOException] {
    Write-Host "The DLL is in use and could not be replaced." -ForegroundColor Red
    Write-Host "Close a-cam.exe and any camera application, then try again."
    exit 1
}
Write-Host "Copied to: $installedDll"

$code = Invoke-RegSvr32 -Path $installedDll
if ($code -ne 0) {
    Write-Host "regsvr32 failed (exit code $code)." -ForegroundColor Red
    exit 1
}

# Verify the key really exists: in silent mode regsvr32 does not always report failure.
$clsidKey = 'HKLM:\Software\Classes\CLSID\{6D0F2B7A-1C84-4F6E-9B35-27A4C1D8E500}\InprocServer32'
if (-not (Test-Path $clsidKey)) {
    Write-Host "The registry key was not created: $clsidKey" -ForegroundColor Red
    exit 1
}
$registered = (Get-ItemProperty $clsidKey).'(default)'
Write-Host "Registered." -ForegroundColor Green
Write-Host "  CLSID -> $registered"
Write-Host ""
Write-Host "Start a-cam.exe and press Start Stream; the camera appears as 'a-Cam (phone)'."
