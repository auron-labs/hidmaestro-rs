[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArchivePath,

    [Parameter(Mandatory)]
    [ValidatePattern('^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$')]
    [string]$Version,

    [Parameter(Mandatory)]
    [string]$ExtractionDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$archive = (Resolve-Path -LiteralPath $ArchivePath).Path
$bundleName = "hidmaestro-rs-$Version-win-x64"
if ([System.IO.Path]::GetFileName($archive) -cne "$bundleName.zip") {
    throw "Unexpected archive name: $archive"
}
$checksumPath = "$archive.sha256"
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
$checksum = "$hash  $bundleName.zip"
if ((Get-Content -LiteralPath $checksumPath -Raw).Trim() -cne $checksum) {
    throw "Checksum does not match $archive"
}

if (Test-Path -LiteralPath $ExtractionDirectory) {
    throw "Extraction directory must be fresh: $ExtractionDirectory"
}
Expand-Archive -LiteralPath $archive -DestinationPath $ExtractionDirectory
$bundle = Join-Path $ExtractionDirectory $bundleName
foreach ($file in @('hidmaestro-mcp.exe', 'hidmaestro-bridge.exe', 'hidmaestro-bridge.dll', 'HIDMaestro.Core.dll', 'LICENSE', 'LICENSE-HIDMAESTRO', 'UNSIGNED.txt')) {
    if (-not (Test-Path -LiteralPath (Join-Path $bundle $file) -PathType Leaf)) {
        throw "Extracted bundle is missing $file"
    }
}
$bridgeVersion = [System.Diagnostics.FileVersionInfo]::GetVersionInfo(
    (Join-Path $bundle 'hidmaestro-bridge.dll')
).ProductVersion
if (($bridgeVersion -split '\+')[0] -cne $Version) {
    throw "Bridge informational version '$bridgeVersion' differs from '$Version'."
}

Remove-Item Env:HIDMAESTRO_PIPE_NAME -ErrorAction SilentlyContinue
Remove-Item Env:HIDMAESTRO_BRIDGE_PATH -ErrorAction SilentlyContinue
& (Join-Path $PSScriptRoot 'smoke-windows.ps1') -BundleDirectory $bundle -ExpectedVersion $Version
