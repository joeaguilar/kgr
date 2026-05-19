<#
.SYNOPSIS
    Install the kgr CLI on Windows from the latest GitHub Release.

.DESCRIPTION
    Downloads a prebuilt kgr binary matching the host architecture
    (x86_64 or arm64), verifies its SHA256 checksum, and installs it
    into a directory on PATH.

.PARAMETER Version
    Pin a specific release tag (e.g. v0.2.0). Defaults to the latest.

.PARAMETER InstallDir
    Install location. Defaults to $env:LOCALAPPDATA\Programs\kgr.

.PARAMETER Repo
    GitHub repo slug. Defaults to joeaguilar/kgr.

.EXAMPLE
    iwr -useb https://raw.githubusercontent.com/joeaguilar/kgr/main/install.ps1 | iex

.EXAMPLE
    .\install.ps1 -Version v0.2.0 -InstallDir C:\tools\kgr
#>

[CmdletBinding()]
param(
    [string]$Version = $env:KGR_VERSION,
    [string]$InstallDir = $env:KGR_INSTALL_DIR,
    [string]$Repo = $(if ($env:KGR_REPO) { $env:KGR_REPO } else { 'joeaguilar/kgr' })
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-Info { param([string]$m) Write-Host "i $m" -ForegroundColor Blue }
function Write-Ok   { param([string]$m) Write-Host "+ $m" -ForegroundColor Green }
function Write-Warn { param([string]$m) Write-Host "! $m" -ForegroundColor Yellow }

function Get-Target {
    $arch = $env:PROCESSOR_ARCHITECTURE
    if (-not $arch) {
        $arch = (Get-CimInstance Win32_Processor).Architecture
    }

    switch -Regex ($arch) {
        '^(AMD64|x86_64|9)$' { return 'x86_64-pc-windows-msvc' }
        '^(ARM64|12)$'       { return 'aarch64-pc-windows-msvc' }
        default {
            throw "Unsupported architecture: $arch"
        }
    }
}

function Resolve-LatestTag {
    param([string]$Repo)

    $url = "https://github.com/$Repo/releases/latest"
    $resp = Invoke-WebRequest -Uri $url -MaximumRedirection 0 -ErrorAction SilentlyContinue
    if ($resp.StatusCode -ne 302 -and $resp.StatusCode -ne 301) {
        if ($resp.BaseResponse.RequestMessage.RequestUri) {
            $final = $resp.BaseResponse.RequestMessage.RequestUri.AbsoluteUri
            return ($final -split '/')[-1]
        }
        throw "Could not resolve latest release tag from $url"
    }

    $location = $resp.Headers.Location
    return ($location -split '/')[-1]
}

function Add-ToUserPath {
    param([string]$Dir)

    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $current) { $current = '' }

    $parts = $current -split ';' | Where-Object { $_ -ne '' }
    if ($parts -contains $Dir) { return $false }

    $new = (@($Dir) + $parts) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $new, 'User')
    $env:Path = "$Dir;$env:Path"
    return $true
}

function Test-InPath {
    param([string]$Dir)

    $parts = $env:Path -split ';' | Where-Object { $_ -ne '' }
    return ($parts -contains $Dir)
}

Write-Host ''
Write-Info 'Installing kgr, the polyglot source dependency knowledge graph'
Write-Host ''

$target = Get-Target
Write-Info "Detected target: $target"

if (-not $Version) {
    $Version = Resolve-LatestTag -Repo $Repo
}
Write-Info "Release: $Version"

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\kgr'
}
$InstallDir = [Environment]::ExpandEnvironmentVariables($InstallDir)

$assetBase = "kgr-$Version-$target"
$zipUrl = "https://github.com/$Repo/releases/download/$Version/$assetBase.zip"
$sumUrl = "$zipUrl.sha256"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

try {
    $zipPath = Join-Path $tmp "$assetBase.zip"
    $sumPath = Join-Path $tmp "$assetBase.zip.sha256"

    Write-Info "Downloading $assetBase.zip"
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing

    try {
        Invoke-WebRequest -Uri $sumUrl -OutFile $sumPath -UseBasicParsing -ErrorAction Stop
        $expected = (Get-Content $sumPath -Raw).Trim().Split()[0].ToLower()
        $actual = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
        if ($expected -ne $actual) {
            throw "Checksum mismatch: expected $expected, got $actual"
        }
        Write-Ok 'Checksum verified.'
    } catch [System.Net.WebException] {
        Write-Warn 'Checksum file not available; skipping verification.'
    }

    Write-Info 'Extracting...'
    Expand-Archive -Path $zipPath -DestinationPath $tmp -Force

    $binSrc = Join-Path $tmp "$assetBase\kgr.exe"
    if (-not (Test-Path $binSrc)) {
        throw 'Extracted archive is missing kgr.exe'
    }

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    }

    $binDst = Join-Path $InstallDir 'kgr.exe'
    Copy-Item -Force $binSrc $binDst
    Write-Ok "Installed $binDst"

    if (-not (Test-InPath $InstallDir)) {
        $added = Add-ToUserPath -Dir $InstallDir
        if ($added) {
            Write-Ok "Added $InstallDir to your User PATH (restart your shell to pick it up)."
        } else {
            Write-Warn "$InstallDir is not in PATH; add it manually if needed."
        }
    }

    Write-Host ''
    try { & $binDst --version } catch { }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

Write-Host ''
Write-Ok 'Done.'
Write-Host ''
Write-Info 'Quick start:'
Write-Host '  kgr                         # dependency tree of the current directory'
Write-Host '  kgr check --format json .   # CI/agent-friendly architecture check'
Write-Host '  kgr agent-info              # machine-readable guide for agents'
Write-Host ''
