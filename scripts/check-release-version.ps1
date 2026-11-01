[CmdletBinding()]
param([string]$Tag)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
$metadata = cargo metadata --locked --format-version 1 --manifest-path (Join-Path $root 'Cargo.toml') | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve locked Cargo metadata.' }
$members = @($metadata.packages | Where-Object { $_.id -in $metadata.workspace_members })
$version = $members[0].version
if (@($members | Where-Object { $_.version -cne $version }).Count) {
    throw 'Workspace package versions differ.'
}
$manifest = Get-Content -LiteralPath (Join-Path $root '.release-please-manifest.json') -Raw | ConvertFrom-Json
$marker = (Get-Content -LiteralPath (Join-Path $root '.release-please-version') -Raw).Trim()
if ($manifest.'.' -cne $version -or $marker -cne $version) {
    throw 'Release Please metadata differs from the workspace version.'
}
if ($Tag -and $Tag -cne "v$version") {
    throw "Release tag '$Tag' differs from workspace version '$version'."
}
$version
