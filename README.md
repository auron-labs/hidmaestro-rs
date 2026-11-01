# hidmaestro-rs

[![CI](https://img.shields.io/github/actions/workflow/status/auron-labs/hidmaestro-rs/ci.yml?branch=main&style=flat-square)](https://github.com/auron-labs/hidmaestro-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/auron-labs/hidmaestro-rs?style=flat-square)](LICENSE)

**Control virtual gamepads from Rust or an AI assistant.**

Create a controller, press buttons, move sticks, and read feedback such as rumble.
Use it to automate controller input and test how applications respond.

**Windows x64 only for controller operation.** The bridge must run as
Administrator; your AI assistant can run without Administrator privileges.
[HIDMaestro](https://github.com/hifihedgehog/HIDMaestro) supplies the underlying
controller implementation. This repository supplies its Rust client, MCP server,
and required .NET bridge. Rust-only builds and driver-free tests also run on
other platforms.

## Get started

The supported installation route is currently a
[source build](docs/getting-started.md#build-your-installation). Follow that guide
to build the complete bundle and copy its contents to `C:\Tools\hidmaestro`.
Do not use upstream HIDMaestro downloads as a substitute for this project's
bridge and MCP server.

Running the bundle requires Windows x64, permission to elevate the bridge, and
an MCP-enabled assistant or Rust application. The .NET runtime is included; you
do not need the compiler toolchains on the machine running the bundle. See
[build requirements](docs/getting-started.md#build-the-bundle-from-source) if
building it yourself.

### Start the bridge as Administrator

Open an **Administrator PowerShell as the same Windows user** as your client:

```powershell
Set-Location C:\Tools\hidmaestro
.\hidmaestro-bridge.exe --pipe hidmaestro-mcp
```

Leave the terminal open and wait for the bridge to announce that it is waiting
for a client. It accepts one local connection and exits when that session ends;
restart it before your next session.

## Use with an AI assistant

In your ordinary MCP client, add a stdio server. For clients using `mcpServers`:

```json
{
  "mcpServers": {
    "hidmaestro": {
      "command": "C:\\Tools\\hidmaestro\\hidmaestro-mcp.exe",
      "env": { "HIDMAESTRO_PIPE_NAME": "hidmaestro-mcp" }
    }
  }
}
```

Complete [first-time driver setup](docs/mcp.md#create-your-first-controller)
explicitly before requesting input. Installation sweeps existing HIDMaestro
devices; it is not implicitly authorized by the task below.

Then ask:

> Create an Xbox 360 controller, tap A for 80 milliseconds, then release all
> inputs and remove only the controller you created.

For an application test, ask the assistant to wait for the application to
enumerate the controller before tapping A. Observe a known response in the
application. Tool success confirms submission, not reception. See the
[MCP walkthrough and recipes](docs/mcp.md) for tool calls and feedback checks.

## Use from Rust

Add the [pinned Git dependency](docs/rust.md), complete driver setup, and start
the elevated bridge above. This complete program taps A and attempts reset and
removal even if the tap fails:

```rust
use hidmaestro::{Buttons, HidMaestro};
use std::time::Duration;

fn main() -> hidmaestro::Result<()> {
    let mut hm = HidMaestro::builder().pipe_name("hidmaestro-mcp").spawn()?;
    hm.load_default_profiles()?;
    let key = hm.create_controller("xbox-360-wired", None)?;

    let tap = hm.controller(&key)?.tap(Buttons::A, Duration::from_millis(80));
    let reset = hm.controller(&key).and_then(|mut pad| pad.reset());
    let remove = hm.remove_controller(&key);
    tap?;
    reset?;
    remove?;
    hm.shutdown();
    Ok(())
}
```

Wait for enumeration before submitting input in a real application test. The
example does not install drivers or perform system-wide cleanup. See
[Rust integration](docs/rust.md) for state, feedback, and session lifetime.

## Limitations and documentation

- The bundle is unsigned. Verify its checksum for integrity, not publisher trust.
  [Local driver certificate installation](docs/getting-started.md#bundle-contents-and-local-driver-trust)
  is a separate trust operation.
- Compatibility varies by controller profile and application input API.
  Successful submission or cached state does not prove the application received input.
- Avoid `remove_all_controllers` for ordinary tasks: it performs system-wide
  cleanup, including attempted driver package/configuration removal.

For more detail:

- [Build and connect](docs/getting-started.md)
- [Operate through MCP](docs/mcp.md) or [integrate with Rust](docs/rust.md)
- [Input and feedback reference](docs/input-reference.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Contribute, test, package, and release](docs/development.md)

For agents: [repository instructions](AGENTS.md) · [MCP operating procedure](docs/agent-usage.md).

## License

This project is [MIT](LICENSE). HIDMaestro is included as a pinned submodule
and retains its own [license](vendor/HIDMaestro/LICENSE).
