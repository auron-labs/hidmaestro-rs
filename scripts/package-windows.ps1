[CmdletBinding()]
param(
    [ValidateSet('Package', 'Stage')]
    [string]$Mode = 'Package',

    [string]$Version,

    [string]$HMRepoRoot,

    [switch]$SkipUpstreamBuild,

    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-WorkspaceVersion {
    param([string]$ManifestPath)

    $manifest = Get-Content -LiteralPath $ManifestPath -Raw
    if ($manifest -match '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }

    throw "Could not read workspace.package.version from $ManifestPath"
}

function Invoke-Checked {
    param(
        [string]$FilePath,
        [string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE"
    }
}

function Copy-DirectoryContents {
    param(
        [string]$Source,
        [string]$Destination
    )

    $items = Get-ChildItem -LiteralPath $Source -Force
    if ($items.Count -eq 0) {
        throw "Publish directory is empty: $Source"
    }
    Copy-Item -LiteralPath $items.FullName -Destination $Destination -Recurse -Force
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($env:OS -ne 'Windows_NT') {
    throw 'Windows packaging requires PowerShell on Windows.'
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = Get-WorkspaceVersion (Join-Path $repositoryRoot 'Cargo.toml')
}
if ($Version -notmatch '^\d+\.\d+\.\d+([-.][0-9A-Za-z.-]+)?$') {
    throw "Version must be a SemVer-like value, got '$Version'."
}

if ([string]::IsNullOrWhiteSpace($HMRepoRoot)) {
    $HMRepoRoot = if ($env:HM_REPO_ROOT) {
        $env:HM_REPO_ROOT
    } else {
        Join-Path $repositoryRoot 'vendor\HIDMaestro'
    }
}
if (-not (Test-Path -LiteralPath $HMRepoRoot -PathType Container)) {
    throw "HIDMaestro source is missing at '$HMRepoRoot'. Initialize submodules or pass -HMRepoRoot."
}
$HMRepoRoot = (Resolve-Path -LiteralPath $HMRepoRoot).Path
$upstreamBuild = Join-Path $HMRepoRoot 'scripts\build_all.cmd'
if (-not (Test-Path -LiteralPath $upstreamBuild -PathType Leaf)) {
    throw "HIDMaestro source is missing at '$HMRepoRoot'. Initialize submodules or pass -HMRepoRoot."
}

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot 'artifacts'
}
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
$bundleName = "hidmaestro-rs-$Version-win-x64"
$stageRoot = Join-Path $OutputDirectory 'stage'
$stageDirectory = Join-Path $stageRoot $bundleName
$bridgePublishDirectory = Join-Path $repositoryRoot 'bridge\HIDMaestro.Bridge\bin\Release\net10.0-windows10.0.26100.0\win-x64\publish'
$mcpExecutable = Join-Path $repositoryRoot 'target\release\hidmaestro-mcp.exe'

if (-not $SkipUpstreamBuild) {
    Write-Host 'Building the pinned HIDMaestro driver and SDK payload...'
    Invoke-Checked $upstreamBuild @()
}

Write-Host 'Publishing the self-contained bridge...'
Invoke-Checked 'dotnet' @(
    'publish',
    (Join-Path $repositoryRoot 'bridge\HIDMaestro.Bridge\HIDMaestro.Bridge.csproj'),
    '--configuration', 'Release',
    '--runtime', 'win-x64',
    '--self-contained', 'true',
    "-p:HMRepoRoot=$HMRepoRoot"
)

Write-Host 'Building hidmaestro-mcp...'
Invoke-Checked 'cargo' @(
    'build',
    '--manifest-path', (Join-Path $repositoryRoot 'Cargo.toml'),
    '--package', 'hidmaestro-mcp',
    '--release',
    '--locked'
)

if (-not (Test-Path -LiteralPath $mcpExecutable -PathType Leaf)) {
    throw "MCP executable was not produced: $mcpExecutable"
}
if (-not (Test-Path -LiteralPath (Join-Path $bridgePublishDirectory 'hidmaestro-bridge.exe') -PathType Leaf)) {
    throw "Bridge executable was not produced: $bridgePublishDirectory"
}
if (-not (Test-Path -LiteralPath (Join-Path $bridgePublishDirectory 'HIDMaestro.Core.dll') -PathType Leaf)) {
    throw 'Bridge publish output does not contain HIDMaestro.Core.dll, which embeds the driver, profile, and transport payloads.'
}

Remove-Item -LiteralPath $stageDirectory -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $stageDirectory -Force | Out-Null
Copy-DirectoryContents $bridgePublishDirectory $stageDirectory
Copy-Item -LiteralPath $mcpExecutable -Destination (Join-Path $stageDirectory 'hidmaestro-mcp.exe') -Force
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -Destination (Join-Path $stageDirectory 'LICENSE') -Force
Copy-Item -LiteralPath (Join-Path $HMRepoRoot 'LICENSE') -Destination (Join-Path $stageDirectory 'LICENSE-HIDMAESTRO') -Force
@"
This Windows x64 bundle is unsigned. Verify the accompanying SHA-256 checksum
before use. HIDMaestro installs its user-mode driver using its own locally
trusted certificate when an elevated bridge first installs the driver.
"@ | Set-Content -LiteralPath (Join-Path $stageDirectory 'UNSIGNED.txt') -NoNewline

if ($Mode -eq 'Stage') {
    Write-Host "Staged bundle: $stageDirectory"
    exit 0
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$archivePath = Join-Path $OutputDirectory "$bundleName.zip"
$checksumPath = "$archivePath.sha256"
Remove-Item -LiteralPath $archivePath, $checksumPath -Force -ErrorAction SilentlyContinue
Compress-Archive -LiteralPath $stageDirectory -DestinationPath $archivePath -CompressionLevel Optimal
$hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $([System.IO.Path]::GetFileName($archivePath))" | Set-Content -LiteralPath $checksumPath -NoNewline

Write-Host "Created archive: $archivePath"
Write-Host "Created checksum: $checksumPath"
