# Driver-free checks. Load only response helpers, never the native smoke body.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-Throws {
    param([scriptblock]$Action, [string]$Message)
    try {
        & $Action | Out-Null
    } catch {
        if ($_.Exception.Message -notmatch $Message) { throw }
        return
    }
    throw "Expected failure matching '$Message'."
}

$version = & (Join-Path $PSScriptRoot 'check-release-version.ps1')
Assert-Throws {
    & (Join-Path $PSScriptRoot 'check-release-version.ps1') -Tag "v$version-invalid"
} 'differs from workspace version'

$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'smoke-windows.ps1'), [ref]$tokens, [ref]$errors
)
if ($errors.Count) { throw ($errors | Out-String) }
$helpers = $ast.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -in @('Read-McpResponse', 'Invoke-McpTool')
}, $false)
foreach ($helper in $helpers) {
    . ([scriptblock]::Create($helper.Extent.Text))
}

$script:RpcTimeoutMilliseconds = 1000
$script:Mcp = [pscustomobject]@{
    StandardOutput = [System.IO.StringReader]::new('{"jsonrpc":"2.0","id":1,"result":{"ok":true}}')
}
$response = Read-McpResponse -RequestId 1 -Method 'test'
if ($response.ok -ne $true) { throw 'Successful response was not returned.' }

$script:Mcp.StandardOutput = [System.IO.StringReader]::new('{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"test error"}}')
Assert-Throws { Read-McpResponse -RequestId 1 -Method 'test' } 'MCP protocol error'

function Invoke-Mcp { return $script:ToolResult }
$script:ToolResult = @{ content = @(@{ type = 'text'; text = 'success' }) }
$result = Invoke-McpTool -Name 'test'
if ($result.content[0].text -ne 'success') { throw 'Successful tool result was not returned.' }
$script:ToolResult = @{ isError = $true; content = @(@{ type = 'text'; text = 'test failure' }) }
Assert-Throws { Invoke-McpTool -Name 'test' } 'test failure'

if ($env:OS -eq 'Windows_NT') {
    # Version validation precedes all upstream/native build invocations.
    Assert-Throws {
        & (Join-Path $PSScriptRoot 'package-windows.ps1') -Version 'not-the-workspace-version'
    } 'differs from workspace.package.version'
}

$previousRunnerEnvironment = $env:RUNNER_ENVIRONMENT
try {
    $env:RUNNER_ENVIRONMENT = 'self-hosted'
    Assert-Throws {
        & (Join-Path $PSScriptRoot 'setup-windows-runner.ps1')
    } 'only for GitHub-hosted Windows'
} finally {
    if ($null -eq $previousRunnerEnvironment) {
        Remove-Item Env:RUNNER_ENVIRONMENT -ErrorAction SilentlyContinue
    } else {
        $env:RUNNER_ENVIRONMENT = $previousRunnerEnvironment
    }
}

$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    $archive = Join-Path $temporary 'hidmaestro-rs-0.1.0-win-x64.zip'
    [System.IO.File]::WriteAllText($archive, 'not an archive')
    [System.IO.File]::WriteAllText("$archive.sha256", 'incorrect checksum')
    $extraction = Join-Path $temporary 'extracted'
    Assert-Throws {
        & (Join-Path $PSScriptRoot 'test-bundle-windows.ps1') -ArchivePath $archive -Version '0.1.0' -ExtractionDirectory $extraction
    } 'Checksum does not match'
    if (Test-Path -LiteralPath $extraction) { throw 'Checksum failure must precede extraction.' }
    Remove-Item -LiteralPath "$archive.sha256"
    Assert-Throws {
        & (Join-Path $PSScriptRoot 'test-bundle-windows.ps1') -ArchivePath $archive -Version '0.1.0' -ExtractionDirectory $extraction
    } 'does not exist'
    if (Test-Path -LiteralPath $extraction) { throw 'Missing checksum must precede extraction.' }
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force
}

Write-Host 'Driver-free PowerShell release checks passed.'
