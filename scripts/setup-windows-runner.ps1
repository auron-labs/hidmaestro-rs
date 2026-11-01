# Provision build tools only on GitHub's disposable Windows runner, never a local host.
$ErrorActionPreference = 'Stop'
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

$required = @(
    "Include\$kitVersion\km\ntddk.h",
    'Include\wdf\umdf\2.15\wdf.h',
    'Lib\wdf\umdf\x64\2.15\WdfDriverStubUm.lib',
    'Lib\wdf\umdf\arm64\2.15\WdfDriverStubUm.lib',
    "Lib\$kitVersion\um\arm64\OneCoreUAP.lib",
    "Lib\$kitVersion\ucrt\arm64\ucrt.lib",
    "bin\$kitVersion\x86\Inf2Cat.exe",
    "bin\$kitVersion\x64\rc.exe",
    "bin\$kitVersion\x64\signtool.exe",
    "bin\$kitVersion\arm64\signtool.exe"
)
$missing = @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $kitRoot $_) -PathType Leaf) })
if ($missing.Count -gt 0) {
    $directory = Join-Path $env:RUNNER_TEMP 'hidmaestro-wdk'
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $installer = Join-Path $directory 'wdksetup.exe'
    $log = Join-Path $directory 'wdk-install.log'
    Write-Host 'Installing the Microsoft WDK 26100.6584 for Visual Studio 2022...'
    # Microsoft's archived 26100 installer; the kit directory uses 10.0.26100.0.
    # https://learn.microsoft.com/windows-hardware/drivers/other-wdk-downloads
    Invoke-WebRequest -Uri 'https://go.microsoft.com/fwlink/?linkid=2335869' -OutFile $installer
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw 'WDK installer does not have a valid Microsoft Authenticode signature.'
    }
    $process = Start-Process -FilePath $installer -ArgumentList @('/quiet', '/norestart', '/features', 'OptionId.WindowsDriverKitComplete', '/log', "`"$log`"") -Wait -PassThru
    if ($process.ExitCode -notin @(0, 3010)) {
        throw "WDK installation failed with exit code $($process.ExitCode). See $log."
    }
    if ($process.ExitCode -eq 3010) {
        Write-Warning 'WDK installer requested a restart; checking required files before continuing.'
    }
}

foreach ($file in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $kitRoot $file) -PathType Leaf)) {
        throw "Required native build file is missing after setup: $file"
    }
}
Write-Host "Native build prerequisites found: $visualStudio; SDK/WDK $kitVersion."
