<#
.SYNOPSIS
    Downloads the scrcpy Windows release into the scrcpy\ folder.

.DESCRIPTION
    scrcpy is not vendored into this repository: it is a separate project, its releases are
    around 90 MB of binaries, and a copy in git would go stale. This script fetches an
    official release instead.

    Two files are what a-Cam actually needs at runtime: scrcpy-server, which is pushed to
    the phone, and adb.exe, which talks to it.

.PARAMETER Version
    A specific scrcpy version, for example 5.0.1. Defaults to the latest release.

.EXAMPLE
    .\fetch-scrcpy.ps1
    .\fetch-scrcpy.ps1 -Version 5.0.1
#>
[CmdletBinding()]
param(
    [string]$Version
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$root = Split-Path $PSScriptRoot -Parent
$target = Join-Path $root 'scrcpy'

if (-not $Version) {
    Write-Host "Looking up the latest scrcpy release..."
    $release = Invoke-RestMethod 'https://api.github.com/repos/Genymobile/scrcpy/releases/latest' `
        -Headers @{ 'User-Agent' = 'a-cam-setup' }
    $Version = $release.tag_name -replace '^v', ''
}
Write-Host "scrcpy version: $Version"

$name = "scrcpy-win64-v$Version.zip"
$url = "https://github.com/Genymobile/scrcpy/releases/download/v$Version/$name"
$zip = Join-Path ([IO.Path]::GetTempPath()) $name

Write-Host "Downloading $url"
Invoke-WebRequest $url -OutFile $zip -UseBasicParsing

if (Test-Path $target) { Remove-Item $target -Recurse -Force }
$staging = Join-Path ([IO.Path]::GetTempPath()) "scrcpy-extract-$([guid]::NewGuid())"
Expand-Archive $zip -DestinationPath $staging -Force

# The archive contains a single top-level folder; move its contents up so that
# scrcpy-server sits directly in scrcpy\, which is where a-Cam looks for it.
$inner = Get-ChildItem $staging -Directory | Select-Object -First 1
if ($inner) {
    Move-Item $inner.FullName $target
} else {
    Move-Item $staging $target
}
Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $zip -Force -ErrorAction SilentlyContinue

$server = Join-Path $target 'scrcpy-server'
if (-not (Test-Path $server)) {
    Write-Host "scrcpy-server is missing from the archive. Extract it manually." -ForegroundColor Red
    exit 1
}
Write-Host "Ready: $target" -ForegroundColor Green
