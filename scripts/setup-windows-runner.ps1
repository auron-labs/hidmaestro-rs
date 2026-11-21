# Provision build tools only on GitHub's disposable Windows runner, never a local host.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest

if ($env:OS -ne 'Windows_NT' -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'This setup script is only for GitHub-hosted Windows Actions runners.'
}

$kitRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
$kitVersion = '10.0.26100.0'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($visualStudio)) {
    throw 'The GitHub Windows image is missing Visual Studio C++ x64/ARM64 tools.'
}
if (-not (Test-Path -LiteralPath (Join-Path $kitRoot "Include\$kitVersion\um\Windows.h"))) {
    throw "The GitHub Windows image is missing SDK $kitVersion. Check its runner image software inventory."
}

$sdkFiles = @(
    "Lib\$kitVersion\um\arm64\OneCoreUAP.lib",
    "Lib\$kitVersion\ucrt\arm64\ucrt.lib",
    "bin\$kitVersion\x64\rc.exe",
    "bin\$kitVersion\x64\signtool.exe",
    "bin\$kitVersion\arm64\signtool.exe"
)
foreach ($file in $sdkFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $kitRoot $file) -PathType Leaf)) {
        throw "Required Windows SDK file is missing: $file"
    }
}

# Match the WDK packages pinned by upstream scripts/check_build_layouts.ps1.
$wdkVersion = '10.0.26100.6584'
$hashes = @{
    x64 = 'c393d03dfb640b5c92f546b32f6770ef68cd3aaf691956e7d66d8e2c28a1b55e'
    arm64 = 'e705b2a63eab891def8f98087666f93e8f21da8e3b5def81a624b83fef5bdae9'
}
$directory = Join-Path $env:RUNNER_TEMP 'hidmaestro-wdk'
New-Item -ItemType Directory -Path $directory -Force | Out-Null
$roots = @{}
foreach ($arch in 'x64', 'arm64') {
    $id = "microsoft.windows.wdk.$arch"
    $name = "$id.$wdkVersion"
    $archive = Join-Path $directory "$name.nupkg"
    Write-Host "Downloading Microsoft WDK $wdkVersion ($arch)..."
    Invoke-WebRequest -Uri "https://api.nuget.org/v3-flatcontainer/$id/$wdkVersion/$name.nupkg" -OutFile $archive
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $hashes[$arch]) {
        throw "WDK package checksum mismatch: $name"
    }
    $destination = Join-Path $directory $name
    [IO.Compression.ZipFile]::ExtractToDirectory($archive, $destination, $true)
    $roots[$arch] = Join-Path $destination 'c'
    $libArch = if ($arch -eq 'arm64') { 'ARM64' } else { 'x64' }
    $wdkFiles = @(
        "Include\$kitVersion\km\hidport.h",
        'Include\wdf\umdf\2.15\wdf.h',
        "Lib\wdf\umdf\$libArch\2.15\WdfDriverStubUm.lib"
    )
    if ($arch -eq 'x64') { $wdkFiles += "bin\$kitVersion\x86\Inf2Cat.exe" }
    foreach ($file in $wdkFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $roots[$arch] $file) -PathType Leaf)) {
            throw "Required WDK file is missing in ${name}: $file"
        }
    }
}

@(
    "VSINSTALLDIR=$($visualStudio.TrimEnd('\'))\"
    "HM_SDK_ROOT=$kitRoot"
    "HM_WDK_ROOT=$($roots.x64)"
    "HM_WDK_ROOT_ARM64=$($roots.arm64)"
    "HM_WDK_VERSION=$kitVersion"
) | Add-Content -LiteralPath $env:GITHUB_ENV -Encoding utf8
Write-Host "Native build prerequisites found: $visualStudio; SDK $kitVersion; NuGet WDK $wdkVersion."
