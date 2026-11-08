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

# Used when the version lookup cannot be made, so a rate-limited CI run still produces a
# working package rather than failing outright.
$fallbackVersion = '5.0.1'

if (-not $Version) {
    $headers = @{ 'User-Agent' = 'a-cam-setup' }
    # Unauthenticated API calls are rate limited per IP, and CI runners share theirs, so
    # use the workflow token when one is in the environment.
    if ($env:GITHUB_TOKEN) { $headers['Authorization'] = "Bearer $env:GITHUB_TOKEN" }
    try {
        Write-Host "Looking up the latest scrcpy release..."
        $release = Invoke-RestMethod 'https://api.github.com/repos/Genymobile/scrcpy/releases/latest' `
            -Headers $headers
        $Version = $release.tag_name -replace '^v', ''
    } catch {
        Write-Host "Lookup failed ($($_.Exception.Message)); using $fallbackVersion." -ForegroundColor Yellow
        $Version = $fallbackVersion
    }
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

# The server refuses to run unless the client reports exactly its own version, so the build
# has to compile it in. Recording it here saves build.rs from launching scrcpy.exe to ask,
# which is a windowed binary and not something a headless CI runner should have to run.
Set-Content -Path (Join-Path $target 'version.txt') -Value $Version -Encoding utf8
Write-Host "Ready: $target" -ForegroundColor Green
