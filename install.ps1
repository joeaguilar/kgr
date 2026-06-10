<#
.SYNOPSIS
    Install or update the kgr CLI on Windows from the latest GitHub Release.

.DESCRIPTION
    Downloads a prebuilt kgr binary matching the host architecture
    (x86_64 or arm64), verifies its SHA256 checksum, and installs it
    into a directory on PATH.

    When -Update is supplied (or the positional `update` argument), the
    installer prefers an existing kgr.exe already on PATH and replaces it
    in-place rather than always writing to $InstallDir. This mirrors the
    `install.sh --update` behavior on Unix.

.PARAMETER Version
    Pin a specific release tag (e.g. v0.2.0). Defaults to the latest.

.PARAMETER InstallDir
    Install location. Defaults to $env:LOCALAPPDATA\Programs\kgr. When
    -Update is set and an existing kgr.exe is found on PATH, that location
    wins over the default so the shell keeps resolving the same binary.

.PARAMETER Repo
    GitHub repo slug. Defaults to joeaguilar/kgr.

.PARAMETER Update
    Update an existing kgr install if found on PATH; otherwise install it.
    The positional value `update` (or `install`) is also accepted, e.g.
    `.\install.ps1 update`.

.EXAMPLE
    iwr -useb https://raw.githubusercontent.com/joeaguilar/kgr/main/install.ps1 | iex

.EXAMPLE
    .\install.ps1 -Version v0.2.0 -InstallDir C:\tools\kgr

.EXAMPLE
    .\install.ps1 -Update

.EXAMPLE
    .\install.ps1 update
#>

[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$Version = $env:KGR_VERSION,
    [string]$InstallDir = $env:KGR_INSTALL_DIR,
    [string]$Repo = $(if ($env:KGR_REPO) { $env:KGR_REPO } else { 'joeaguilar/kgr' }),
    [switch]$Update,
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$Action
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-Info { param([string]$m) Write-Host "i $m" -ForegroundColor Blue }
function Write-Ok   { param([string]$m) Write-Host "+ $m" -ForegroundColor Green }
function Write-Warn { param([string]$m) Write-Host "! $m" -ForegroundColor Yellow }
function Write-Err  { param([string]$m) Write-Host "x $m" -ForegroundColor Red }

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

function Get-ExistingKgrPath {
    $cmd = Get-Command kgr.exe -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Path -and (Test-Path $cmd.Path)) {
        return $cmd.Path
    }
    return $null
}

function Get-ExistingKgrInDir {
    param([string]$Dir)

    if (-not $Dir) { return $null }

    $candidate = Join-Path $Dir 'kgr.exe'
    if (Test-Path $candidate) {
        return $candidate
    }

    return $null
}

function Show-ExistingVersion {
    param([string]$BinPath)

    if (-not $BinPath) { return }

    try {
        $ver = & $BinPath --version 2>$null
        if ($ver) {
            Write-Info "Current install: $ver ($BinPath)"
        } else {
            Write-Info "Current install: $BinPath"
        }
    } catch {
        Write-Info "Current install: $BinPath"
    }
}

$ActionMode = if ($Update) { 'update' } else { 'install' }
if ($Action) {
    foreach ($arg in $Action) {
        switch -Regex ($arg) {
            '^(--update|update)$'   { $ActionMode = 'update' }
            '^(--install|install)$' { $ActionMode = 'install' }
            '^(-h|--help)$' {
                $scriptPath = $MyInvocation.MyCommand.Path
                if ($scriptPath -and (Test-Path $scriptPath)) {
                    Get-Help $scriptPath -Detailed
                } else {
                    Write-Host 'Usage:'
                    Write-Host '  .\install.ps1 [-Update] [-Version <tag>] [-InstallDir <path>] [-Repo <slug>]'
                    Write-Host '  .\install.ps1 update      # positional form, mirrors install.sh'
                    Write-Host ''
                    Write-Host 'Environment overrides:'
                    Write-Host '  KGR_VERSION       Pin a specific release tag (defaults to latest).'
                    Write-Host '  KGR_INSTALL_DIR   Override the install directory.'
                    Write-Host '  KGR_REPO          Override the GitHub repo slug.'
                }
                exit 0
            }
            default {
                Write-Err "Unknown argument: $arg"
                exit 1
            }
        }
    }
}

Write-Host ''
if ($ActionMode -eq 'update') {
    Write-Info 'Updating kgr, the polyglot source dependency knowledge graph'
} else {
    Write-Info 'Installing kgr, the polyglot source dependency knowledge graph'
}
Write-Host ''

$target = Get-Target
Write-Info "Detected target: $target"

if (-not $Version) {
    $Version = Resolve-LatestTag -Repo $Repo
}
Write-Info "Release: $Version"

$existingKgr = Get-ExistingKgrPath
if (-not $InstallDir) {
    if ($existingKgr) {
        $InstallDir = Split-Path -Parent $existingKgr
        if ($ActionMode -eq 'install') {
            Write-Info "Existing kgr.exe found on PATH; installing alongside it at $InstallDir"
        }
    } else {
        $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\kgr'
    }
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

    $hasChecksum = $true
    try {
        Invoke-WebRequest -Uri $sumUrl -OutFile $sumPath -UseBasicParsing -ErrorAction Stop
    } catch {
        $statusCode = $null
        if ($_.Exception.Response -and $_.Exception.Response.StatusCode) {
            $statusCode = [int]$_.Exception.Response.StatusCode
        }
        if ($statusCode -eq 404) {
            $hasChecksum = $false
            Write-Warn 'Checksum file not available (HTTP 404); skipping verification.'
        } else {
            throw
        }
    }

    if ($hasChecksum) {
        $expected = (Get-Content $sumPath -Raw).Trim().Split()[0].ToLower()
        $actual = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
        if ($expected -ne $actual) {
            throw "Checksum mismatch: expected $expected, got $actual"
        }
        Write-Ok 'Checksum verified.'
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
    $existingBefore = Get-ExistingKgrInDir -Dir $InstallDir
    Show-ExistingVersion -BinPath $existingBefore

    if ($existingBefore) {
        Write-Info "Updating $binDst"
    } else {
        Write-Info "Installing to $InstallDir"
    }

    Copy-Item -Force $binSrc $binDst
    if ($existingBefore) {
        Write-Ok "Updated $binDst"
    } else {
        Write-Ok "Installed $binDst"
    }

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
