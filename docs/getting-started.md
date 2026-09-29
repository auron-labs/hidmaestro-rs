# Build and connect

[README](../README.md) · Next: [create a controller through MCP](mcp.md) or [Rust](rust.md)

## Requirements

### Run a complete bundle

- Windows x64 and permission to run the bridge as Administrator for driver
  installation and controller creation.
- An MCP client supporting this server's [implemented protocol flows](mcp.md#protocol-and-process-lifetime),
  or a Rust application using the client library.
- The complete bundle directory. Its .NET runtime is self-contained: Rust,
  the .NET SDK, Visual Studio, and the WDK are build requirements, not runtime
  requirements for a bundle user.

The bridge targets `net10.0-windows10.0.26100.0` and publishes for `win-x64`.
Upstream's ARM64 support does not make this repository's bridge an ARM64 build.

### Build the bundle from source

Use Windows x64 with these tools available:

| Requirement | Setup detail |
|---|---|
| Git | Clone the pinned submodule recursively. |
| Rust | Use the MSVC Windows toolchain. `mise.toml` pins 1.98.1; CI uses rolling `stable`. |
| .NET SDK | 10.0; the bridge and upstream SDK target .NET 10. |
| Visual Studio | 2022 or newer, with the C++ workload, MSVC x64 tools, ARM64 cross-compiler and ARM64 CRT libraries. |
| Windows SDK and WDK | 10.0.26100.0, including UMDF 2.15 headers/libraries and signing/catalog tools for the upstream resource pack. |
| PowerShell | PowerShell 7 (`pwsh`) for repository commands; upstream also invokes Windows PowerShell (`powershell`). |
| Network/cache | Cargo/NuGet packages and upstream's pinned usbip-win2 installers must be available. |

Install the native tools using the Visual Studio Installer and Windows SDK/WDK
installers. The pinned native scripts locate `vcvarsall.bat` under
`C:\Program Files\Microsoft Visual Studio` and the kits under
`C:\Program Files (x86)\Windows Kits\10`. Both x64 and ARM64 payloads are required
by upstream `build_all.cmd`, even though this repository publishes an x64 bundle.
The SDK resource pack also uses the 10.0.26100.0 tool directory explicitly.

The workspace declares Rust 1.75 as its minimum supported Rust version (MSRV),
but CI does not test that version. Use the configured 1.98.1 toolchain or current
stable rather than treating the manifest declaration as verified compatibility.
`mise install` installs the tools declared in [mise.toml](../mise.toml), not the
Visual Studio/SDK/WDK prerequisites.

### Run Rust-only checks

Linux, macOS, or Windows with Rust and Python 3 can run the driver-free checks.
No .NET SDK, WDK, or actual controller is needed. On Unix, MCP process tests use
`python3`; on Windows they use `py -3`. Library process tests look for `python3`
then `python`. See [contributor checks](development.md#run-rust-only-checks).

## Build your installation

As of 29 September 2026, the [repository releases](https://github.com/auron-labs/hidmaestro-rs/releases)
contain no published bundle. Use the source route below. A release workflow or
the `0.1.0` manifest version does not imply that a ZIP is available.

Starting in your source parent directory, in PowerShell:

```powershell
git clone --recurse-submodules https://github.com/auron-labs/hidmaestro-rs.git
Set-Location hidmaestro-rs
```

For an existing clone missing upstream sources, start in the repository root:

```powershell
git submodule update --init --recursive
git submodule status
```

The pinned HIDMaestro commit is `942e25aa9ce4e93dc601f59a796f7432809fc41b`.
From the repository root, build and stage:

```powershell
pwsh -File scripts/package-windows.ps1 -Mode Stage
Resolve-Path .\artifacts\stage\hidmaestro-rs-0.1.0-win-x64
```

The script builds upstream native payloads and the SDK, publishes the
self-contained bridge, builds the release MCP executable, then prints
`Staged bundle:` followed by the absolute directory. At the current manifest
version, that directory is `artifacts\stage\hidmaestro-rs-0.1.0-win-x64`.
Use the printed path if working on a later version. `cargo build` produces only
the Rust components. Build option details belong in [development](development.md#windows-bundle).

## Bundle contents and local driver trust

Keep the entire directory together when moving it. It includes
`hidmaestro-mcp.exe`, `hidmaestro-bridge.exe`, the self-contained .NET runtime,
`HIDMaestro.Core.dll`, `LICENSE`, `LICENSE-HIDMAESTRO`, and `UNSIGNED.txt`.
The SDK assembly embeds driver, profile, and transport payloads. Copying only
the two executables is insufficient.

The packaging script produces unsigned output. If you receive a ZIP and its
`.sha256`, compare `Get-FileHash -Algorithm SHA256` with the hash in that file
before extracting the whole archive. A match checks integrity against that
checksum; it does not authenticate an unsigned publisher.

Driver installation is a separate trust operation. The pinned SDK creates a
self-signed `HIDMaestroTestCert` when absent, stores it in `LocalMachine\My`,
and adds the new certificate to `LocalMachine\Root` and
`LocalMachine\TrustedPublisher`. It signs/catalogs the extracted driver and
installs it through Windows. This does not sign or authenticate the bundle.
Installation also sweeps existing HIDMaestro virtual devices before deployment;
do it as setup, not as a routine response to an input error.

## Run the bridge as Administrator

Use the named-pipe broker so only the bridge needs elevation. Open an
Administrator PowerShell **as the same Windows user** as the ordinary client.
Starting in the repository root after staging:

```powershell
Set-Location .\artifacts\stage\hidmaestro-rs-0.1.0-win-x64
.\hidmaestro-bridge.exe --pipe hidmaestro-mcp
```

For a moved bundle, start in its directory and run the second command. Leave
the terminal open until the session ends. Wait for the bridge's message that
it is waiting for one client before connecting.

The broker accepts one connection and exits after disconnection or `shutdown`.
It is not a persistent multi-client service: start a new broker before the next
connection. It uses a local Windows named pipe restricted to the current user;
running the client under a different account will not work. It is not a network
transport or a sandbox: connected tools still operate the elevated bridge.

A pipe name must be 1–128 ASCII characters, start with a letter or digit, and
contain only letters, digits, `-`, `_`, or `.`. Supply `hidmaestro-mcp`, not a
filesystem path or a `\\.\pipe\`-prefixed string. Both ends construct the pipe
path themselves. A connection attempt does not start or wait indefinitely for
a missing broker.

## Connect an ordinary MCP client

For clients that use `mcpServers`, this is a stdio launch configuration shape,
not a universal configuration file. Substitute the absolute path to your staged
or moved bundle:

```json
{
  "mcpServers": {
    "hidmaestro": {
      "command": "C:\\src\\hidmaestro-rs\\artifacts\\stage\\hidmaestro-rs-0.1.0-win-x64\\hidmaestro-mcp.exe",
      "env": {
        "HIDMAESTRO_PIPE_NAME": "hidmaestro-mcp"
      }
    }
  }
}
```

The client launches the MCP executable and exchanges protocol messages over
stdin/stdout. Tool discovery does not connect to the bridge. The first
bridge-dependent call, such as `status`, connects lazily. Follow the
[first-controller walkthrough](mcp.md#create-your-first-controller) to install
the driver if missing, load profiles, send input, observe the application, and
clean up.

## Spawn the bridge directly

Without a configured pipe, the Rust client inside the MCP server launches the
bridge as a child over stdio. It inherits the launching process's privileges;
there is no automatic elevation prompt. For device creation this means the
launching context must already be elevated. Prefer the broker when your MCP
client should run ordinarily.

Connection/discovery order is:

1. Explicit Rust builder `pipe_name`, otherwise nonempty `HIDMAESTRO_PIPE_NAME`.
   A configured pipe selects connection mode before any executable lookup.
2. Otherwise, explicit builder `bridge_path`.
3. Nonempty `HIDMAESTRO_BRIDGE_PATH`.
4. `hidmaestro-bridge.exe` beside the **current executable**.
5. `hidmaestro-bridge.exe` on `PATH`.

The sibling lookup does not use the shell's working directory. For a Rust
application it means beside that application's executable, not beside its
`Cargo.toml`. Set an absolute bridge path if the bundle lives elsewhere.
Builder `arg`/`env` configure a spawned child; they do not configure an external
broker. See [Rust connection patterns](rust.md#choose-a-connection).

## Runtime environment variables

| Variable | Meaning and default |
|---|---|
| `HIDMAESTRO_PIPE_NAME` | Connect to an already-running local broker; empty/unset selects executable spawning unless the Rust builder specifies a pipe. |
| `HIDMAESTRO_BRIDGE_PATH` | Bridge executable path in spawn mode; empty/unset falls through to sibling then `PATH`. Ignored in pipe mode. |
| `HIDMAESTRO_MCP_INACTIVITY_MS` | Optional positive integer milliseconds of MCP stdin inactivity before attempting to reset session controllers to neutral. Unset, zero, invalid, negative, or overflowing values disable it. |

The inactivity reset is disabled by default and runs once per idle period; it
does not remove devices. Any received line restarts idle handling. The dispatcher
is synchronous, so the reset cannot run during an active tool call. Cancellation
notifications cannot interrupt that call either. Use short taps/sequences and
explicit cleanup rather than relying on this setting as an emergency release.
