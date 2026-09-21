[CmdletBinding()]
param(
    [string]$BundleDirectory = (Join-Path $PSScriptRoot '..\artifacts\stage'),
    [ValidateRange(5, 120)]
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Require-WindowsX64Administrator {
    if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) {
        throw 'This smoke test requires an elevated 64-bit PowerShell session on Windows x64.'
    }

    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'This smoke test installs a driver. Re-run it from an elevated Administrator PowerShell session.'
    }
}

function Resolve-BundleDirectory {
    param([string]$Path)

    $resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    if (Test-Path -LiteralPath (Join-Path $resolved 'hidmaestro-mcp.exe') -PathType Leaf) {
        return $resolved
    }

    $bundles = @(
        Get-ChildItem -LiteralPath $resolved -Directory |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'hidmaestro-mcp.exe') -PathType Leaf }
    )
    if ($bundles.Count -ne 1) {
        throw "Expected exactly one staged bundle containing hidmaestro-mcp.exe under '$resolved'. Pass -BundleDirectory with the bundle directory explicitly."
    }
    return $bundles[0].FullName
}

function Start-Mcp {
    param([string]$Executable, [string]$WorkingDirectory)

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $script:Mcp = [System.Diagnostics.Process]::new()
    $script:Mcp.StartInfo = $startInfo
    if (-not $script:Mcp.Start()) {
        throw "Could not start MCP executable '$Executable'."
    }
    $script:Mcp.add_ErrorDataReceived({
        param($sender, $event)
        if ($null -ne $event.Data) {
            $script:McpDiagnostics.Enqueue($event.Data)
        }
    })
    $script:Mcp.BeginErrorReadLine()
}

function Read-McpResponse {
    param([int]$RequestId, [string]$Method)

    $lineTask = $script:Mcp.StandardOutput.ReadLineAsync()
    if (-not $lineTask.Wait($script:RpcTimeoutMilliseconds)) {
        throw "Timed out after $script:RpcTimeoutMilliseconds ms waiting for MCP response to '$Method' (request $RequestId)."
    }
    $line = $lineTask.Result
    if ($null -eq $line) {
        throw "MCP stdout closed while waiting for '$Method' (request $RequestId)."
    }
    try {
        $response = $line | ConvertFrom-Json -Depth 30
    } catch {
        throw "MCP returned invalid JSON for '$Method': $line"
    }
    if ($response.id -ne $RequestId) {
        throw "MCP response id '$($response.id)' did not match request $RequestId for '$Method'."
    }
    if ($null -ne $response.error) {
        throw "MCP protocol error for '$Method': $($response.error.code) $($response.error.message)"
    }
    return $response.result
}

function Invoke-Mcp {
    param([string]$Method, [hashtable]$Parameters = @{})

    $requestId = $script:NextRequestId
    $script:NextRequestId++
    $request = [ordered]@{
        jsonrpc = '2.0'
        id = $requestId
        method = $Method
    }
    if ($Parameters.Count -gt 0) {
        $request.params = $Parameters
    }
    $script:Mcp.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 30))
    $script:Mcp.StandardInput.Flush()
    return Read-McpResponse -RequestId $requestId -Method $Method
}

function Invoke-McpTool {
    param([string]$Name, [hashtable]$Arguments = @{})

    $result = Invoke-Mcp -Method 'tools/call' -Parameters @{
        name = $Name
        arguments = $Arguments
    }
    if ($result.isError -eq $true) {
        $message = @($result.content | ForEach-Object { $_.text }) -join [Environment]::NewLine
        throw "MCP tool '$Name' failed: $message"
    }
    return $result
}

function Get-XInputStates {
    $states = @()
    foreach ($slot in 0..3) {
        $state = [HidMaestroSmoke.XINPUT_STATE]::new()
        $status = [HidMaestroSmoke.XInput]::XInputGetState([uint32]$slot, [ref]$state)
        if ($status -eq 0) {
            $states += [pscustomobject]@{
                Slot = $slot
                Buttons = [uint16]$state.Gamepad.wButtons
            }
        } elseif ($status -ne 1167) { # ERROR_DEVICE_NOT_CONNECTED
            throw "XInputGetState($slot) failed with Win32 error $status."
        }
    }
    return $states
}

function Wait-ForNewXInputController {
    param([int[]]$BaselineSlots)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $states = @(Get-XInputStates)
        $newSlots = @($states | Where-Object { $BaselineSlots -notcontains $_.Slot } | ForEach-Object { $_.Slot })
        if ($newSlots.Count -gt 0) {
            return $newSlots
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "No newly connected XInput controller appeared in an unoccupied slot 0..3 within $TimeoutSeconds seconds after create_controller. Existing slots were: $($BaselineSlots -join ', ')."
}

function Wait-ForButtonState {
    param([bool]$Pressed, [int[]]$Slots)

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $states = @(Get-XInputStates)
        $targetStates = @($states | Where-Object { $Slots -contains $_.Slot })
        $matching = @($targetStates | Where-Object { ($_.Buttons -band 0x1000) -ne 0 }) # XINPUT_GAMEPAD_A
        if ($Pressed -and $matching.Count -gt 0) {
            return @($matching | ForEach-Object { $_.Slot })
        }
        if (-not $Pressed -and $targetStates.Count -eq $Slots.Count -and $matching.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    $state = if ($Pressed) { 'pressed' } else { 'released' }
    $connectedSlots = @($targetStates | ForEach-Object { $_.Slot })
    throw "XInput did not observe A $state in target slot(s) $($Slots -join ', ') within $TimeoutSeconds seconds. Target slot(s) still connected: $($connectedSlots -join ', ')."
}

function Invoke-CleanupTool {
    param([string]$Name, [hashtable]$Arguments = @{})

    try {
        if ($null -ne $script:Mcp -and -not $script:Mcp.HasExited) {
            Invoke-McpTool -Name $Name -Arguments $Arguments | Out-Null
        }
    } catch {
        Write-Warning "Cleanup tool '$Name' failed: $($_.Exception.Message)"
    }
}

function Write-McpDiagnostics {
    if ($script:McpDiagnostics.Count -gt 0) {
        Write-Warning "MCP stderr: $(@($script:McpDiagnostics) -join [Environment]::NewLine)"
    }
}

Require-WindowsX64Administrator
$bundle = Resolve-BundleDirectory -Path $BundleDirectory
$mcpExecutable = Join-Path $bundle 'hidmaestro-mcp.exe'
if (-not (Test-Path -LiteralPath (Join-Path $bundle 'hidmaestro-bridge.exe') -PathType Leaf)) {
    throw "Bundle '$bundle' does not contain hidmaestro-bridge.exe. Stage a complete bundle first."
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace HidMaestroSmoke {
    [StructLayout(LayoutKind.Sequential)]
    public struct XINPUT_GAMEPAD {
        public ushort wButtons;
        public byte bLeftTrigger;
        public byte bRightTrigger;
        public short sThumbLX;
        public short sThumbLY;
        public short sThumbRX;
        public short sThumbRY;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct XINPUT_STATE {
        public uint dwPacketNumber;
        public XINPUT_GAMEPAD Gamepad;
    }

    public static class XInput {
        [DllImport("xinput1_4.dll", CallingConvention = CallingConvention.StdCall)]
        public static extern uint XInputGetState(uint dwUserIndex, out XINPUT_STATE pState);
    }
}
'@

$script:Mcp = $null
$script:McpDiagnostics = [System.Collections.Concurrent.ConcurrentQueue[string]]::new()
$script:NextRequestId = 1
$script:RpcTimeoutMilliseconds = [Math]::Min($TimeoutSeconds * 1000, 30000)
$controller = $null

try {
    Write-Host "Starting MCP server from $bundle"
    Start-Mcp -Executable $mcpExecutable -WorkingDirectory $bundle
    Invoke-Mcp -Method 'initialize' -Parameters @{ protocolVersion = '2025-03-26' } | Out-Null

    Invoke-CleanupTool -Name 'remove_all_controllers'
    $baselineSlots = @(Get-XInputStates | ForEach-Object { $_.Slot })
    if ($baselineSlots.Count -eq 4) {
        throw 'All XInput slots 0..3 are already occupied after removing HIDMaestro controllers. Disconnect a controller and retry so this smoke test can identify its own device.'
    }
    $status = Invoke-McpTool -Name 'status'
    if ($status.structuredContent.driver_installed -ne $true) {
        Write-Host 'Installing HIDMaestro driver...'
        Invoke-McpTool -Name 'install_driver' | Out-Null
    }
    Invoke-McpTool -Name 'load_profiles' | Out-Null
    Invoke-McpTool -Name 'create_controller' -Arguments @{ profile_id = 'xbox-360-wired' } | Out-Null

    $listResult = Invoke-McpTool -Name 'list_controllers'
    $controllers = @($listResult.structuredContent.items)
    $createdControllers = @($controllers | Where-Object { $_.profile_id -eq 'xbox-360-wired' })
    if ($createdControllers.Count -gt 0) {
        $controller = $createdControllers[-1].key
    }
    if ([string]::IsNullOrWhiteSpace($controller)) {
        throw 'create_controller succeeded but list_controllers did not return an xbox-360-wired controller key.'
    }

    $targetSlots = @(Wait-ForNewXInputController -BaselineSlots $baselineSlots)
    Write-Host "XInput newly enumerated controller slot(s): $($targetSlots -join ', ')"
    Invoke-McpTool -Name 'hold_buttons' -Arguments @{ controller = $controller; buttons = 'a' } | Out-Null
    $pressedSlots = @(Wait-ForButtonState -Pressed $true -Slots $targetSlots)
    Write-Host "XInput observed A pressed in slot(s): $($pressedSlots -join ', ')"
    Invoke-McpTool -Name 'release_buttons' -Arguments @{ controller = $controller; buttons = 'a' } | Out-Null
    Wait-ForButtonState -Pressed $false -Slots $targetSlots
    Write-Host 'XInput observed A released.'
} finally {
    if ($null -ne $controller) {
        Invoke-CleanupTool -Name 'reset_controller' -Arguments @{ controller = $controller }
    }
    Invoke-CleanupTool -Name 'remove_all_controllers'
    Invoke-CleanupTool -Name 'shutdown'
    if ($null -ne $script:Mcp) {
        try { $script:Mcp.StandardInput.Close() } catch { Write-Warning "Could not close MCP stdin: $($_.Exception.Message)" }
        if (-not $script:Mcp.WaitForExit($script:RpcTimeoutMilliseconds)) {
            Write-Warning 'MCP did not exit after shutdown; terminating it.'
            $script:Mcp.Kill()
            $script:Mcp.WaitForExit()
        }
        $script:Mcp.Dispose()
    }
    Write-McpDiagnostics
}
