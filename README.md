# hidmaestro-rs

[![CI](https://img.shields.io/github/actions/workflow/status/auron-labs/hidmaestro-rs/ci.yml?branch=main&style=flat-square)](https://github.com/auron-labs/hidmaestro-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/auron-labs/hidmaestro-rs?style=flat-square)](LICENSE)

hidmaestro-rs creates virtual game controllers on Windows. Use it from Rust tests
or an MCP-enabled assistant to press buttons, move sticks, and read feedback sent
by an application. For example, test that a game's menu responds to a controller
button, then check its rumble output.

[HIDMaestro](https://github.com/hifihedgehog/HIDMaestro) supplies the underlying
controller implementation. This repository provides the Rust client, a server
for Model Context Protocol (MCP, a protocol through which assistants discover and
call tools), and a .NET bridge to HIDMaestro. The bridge is required for actual
controller operation even when using only the Rust crate.

Controller operation requires **Windows x64**. Run the bridge as Administrator
for driver installation and controller creation. Rust builds and driver-free
tests also run on other platforms. Application compatibility depends on the
chosen profile and the application's input API; verify it in your target.

## Install on Windows x64

As of 29 September 2026, this repository has no published release bundle. Build
from source; the [releases page](https://github.com/auron-labs/hidmaestro-rs/releases)
is where future assets will appear. Upstream HIDMaestro downloads do not contain
this project's MCP server and bridge.

For the build, install Rust, .NET SDK 10, PowerShell 7 (`pwsh`), Visual Studio
2022+ C++ tools for **x64 and ARM64**, and Windows SDK/WDK 10.0.26100.0. The pinned
upstream builds both native payloads. See [prerequisite setup](docs/getting-started.md#requirements)
for details. A completed self-contained bundle needs neither the SDK nor WDK on
the machine running it.

From a Windows PowerShell terminal in your source parent directory:

```powershell
git clone --recurse-submodules https://github.com/auron-labs/hidmaestro-rs.git
Set-Location hidmaestro-rs
pwsh -File scripts/package-windows.ps1 -Mode Stage
Resolve-Path .\artifacts\stage\hidmaestro-rs-0.1.0-win-x64
```

Keep the entire staged directory together. `cargo build` alone does not build
the bridge or driver payload. Record the absolute staged path printed above.

### Elevated broker workflow

Open an **Administrator PowerShell** as the same Windows user as your MCP
client. Change to the staged directory just printed, then run:

```powershell
.\hidmaestro-bridge.exe --pipe hidmaestro-mcp
```

Leave this terminal open. The bridge waits for one local client. This extra step
lets the assistant itself run without Administrator privileges. After that
connection ends, the bridge exits; start it again for a new session.

### MCP configuration

In your **ordinary MCP client**, configure a stdio server. This is the shape for
clients using `mcpServers`; replace the example absolute path with your staged
executable's path:

```json
{
  "mcpServers": {
    "hidmaestro": {
      "command": "C:\\src\\hidmaestro-rs\\artifacts\\stage\\hidmaestro-rs-0.1.0-win-x64\\hidmaestro-mcp.exe",
      "env": { "HIDMAESTRO_PIPE_NAME": "hidmaestro-mcp" }
    }
  }
}
```

Let the client launch the server and discover its tools. Running the executable
alone just waits for MCP input. [Connection details and direct spawning](docs/getting-started.md#connect-an-ordinary-mcp-client)
cover other setups.

### Create your first controller

Call these tools through the client in order. Each cell is a tool name and its
JSON argument object:

| Step | Tool and arguments |
|---|---|
| Check readiness | `status` `{}` |
| If `driver_installed` is false, perform initial setup | `install_driver` `{}` |
| Load and inspect the profile | `load_profiles` `{}`, then `get_profile` `{"profile_id":"xbox-360-wired"}` |
| Create a deployable profile | `create_controller` `{"profile_id":"xbox-360-wired"}` |
| Obtain its key | `list_controllers` `{}`; read `structuredContent.items` |
| Tap A | `tap_buttons` `{"controller":"c1","buttons":"a","duration_ms":80}` |
| Move, then recenter | `set_sticks` `{"controller":"c1","left_x":0.75}`, then `set_sticks` `{"controller":"c1","left_x":0.5}` |
| Release everything and unplug | `reset_controller` `{"controller":"c1"}`, then `remove_controller` `{"controller":"c1"}` |
| End this bridge session | `shutdown` `{}`; disconnect the MCP server in your client |

Use the actual returned key everywhere: `c1` is illustrative. Before sending
input, let the target application enumerate the controller and open its input
display or a menu where A has a known effect. Observe that effect; tool success
and `get_state` only confirm submission/local state, not application reception.
`wait_for_output_events` can observe feedback when the application sends it.

Perform cleanup even if the application does not respond. Avoid
`remove_all_controllers` for this task: it performs system-wide HIDMaestro cleanup.
Driver installation also sweeps existing HIDMaestro devices, so use it only for
the requested setup. See [MCP recipes](docs/mcp.md) for bounded sequences and
feedback checks.

## Rust quick start

Use the [Rust integration guide](docs/rust.md) for a pinned Git dependency and a
complete program. `create_controller()` returns a key; obtain a borrowed
controller view with `hm.controller(&key)?`. The same elevated bridge can serve
a Rust client over a named pipe.

## Bundle contents and unsigned status

The bundle includes the self-contained bridge runtime, MCP executable, upstream
SDK/payload, licenses, and `UNSIGNED.txt`. It is unsigned. A checksum checks file
integrity, not publisher trust. [Local driver certificate installation](docs/getting-started.md#bundle-contents-and-local-driver-trust)
is separate from distribution signing.

## Troubleshooting

Start with [symptom-to-fix guidance](docs/troubleshooting.md) for connection,
profile, stuck-input, and feedback problems.

## Source development

- [Build and connect](docs/getting-started.md)
- [Operate through MCP](docs/mcp.md) or [integrate with Rust](docs/rust.md)
- [Look up input and feedback semantics](docs/input-reference.md)
- [Contribute, test, package, and release](docs/development.md)

For agents: [repository instructions](AGENTS.md) · [MCP operating procedure](docs/agent-usage.md).

## License

This project is [MIT](LICENSE). HIDMaestro is included as a pinned submodule
and retains its own [license](vendor/HIDMaestro/LICENSE).
