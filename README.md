# hidmaestro-rs

Control [HIDMaestro](https://github.com/hifihedgehog/HIDMaestro) virtual game
controllers from Rust or an MCP client. HIDMaestro presents controllers as real
Windows hardware to DirectInput, XInput, SDL3, browser Gamepad, and WGI/GameInput.
This project is for end-to-end tests that need a controller and for agents that
need to drive one.

| Component | Purpose |
|---|---|
| `hidmaestro` | Rust client: install the driver, create controllers, and submit state. |
| `hidmaestro-mcp.exe` | MCP stdio server for agent tools such as `press_buttons` and `set_sticks`. |
| `hidmaestro-bridge.exe` | Self-contained .NET host for the HIDMaestro SDK and driver payload. |

## Install on Windows x64

Download the Windows x64 ZIP and its `.sha256` file from the [latest
release](https://github.com/auron-labs/hidmaestro-rs/releases/latest). Verify
the checksum before extracting:

```powershell
Get-FileHash .\hidmaestro-rs-<version>-win-x64.zip -Algorithm SHA256
Get-Content .\hidmaestro-rs-<version>-win-x64.zip.sha256
Expand-Archive .\hidmaestro-rs-<version>-win-x64.zip -DestinationPath .\hidmaestro
```

Run `hidmaestro-mcp.exe` from the extracted directory. It finds the sibling
`hidmaestro-bridge.exe` automatically; no `PATH` or environment variable is
needed for the bundle.

```powershell
Set-Location .\hidmaestro\hidmaestro-rs-<version>-win-x64
.\hidmaestro-mcp.exe
```

The bridge and virtual driver are Windows x64 only. Installing the driver or
creating controllers requires Administrator elevation.

### MCP configuration

Point an MCP client at the extracted executable:

```json
{
  "mcpServers": {
    "hidmaestro": {
      "command": "C:\\tools\\hidmaestro\\hidmaestro-rs-<version>-win-x64\\hidmaestro-mcp.exe"
    }
  }
}
```

The usual agent flow is `install_driver` → `load_profiles` → `list_profiles`
→ `create_controller` with `profile_id: "xbox-360-wired"` → input tools such
as `press_buttons`, `set_sticks`, and `set_dpad`. `drain_output_events` reports
rumble, LED, and force-feedback output from games. Use `tap_buttons`,
`hold_buttons`, or bounded `run_action_sequence` for timed input.

Set `HIDMAESTRO_MCP_INACTIVITY_MS` to a positive duration in milliseconds to
neutralize live controllers after MCP stdin inactivity. It is disabled by
default.

### Elevated broker workflow

To keep the MCP client unprivileged, start only the bridge from an **elevated**
PowerShell or Command Prompt. Choose a safe pipe name: it starts with a letter
or digit and then contains only letters, digits, `-`, `_`, or `.`.

```powershell
.\hidmaestro-bridge.exe --pipe hidmaestro-mcp
```

Then give the regular MCP process the same pipe name:

```json
{
  "mcpServers": {
    "hidmaestro": {
      "command": "C:\\tools\\hidmaestro\\hidmaestro-rs-<version>-win-x64\\hidmaestro-mcp.exe",
      "env": { "HIDMAESTRO_PIPE_NAME": "hidmaestro-mcp" }
    }
  }
}
```

The bridge accepts one client from the same Windows user, then exits when that
client disconnects or sends `shutdown`. Do not pass a path or a `\\.\pipe\`
prefix as the name.

## Rust quick start

`HidMaestro::spawn()` prefers an explicit builder path, then
`HIDMAESTRO_BRIDGE_PATH`, then a bridge next to the current executable, and
finally `PATH`.

```rust
use hidmaestro::{Buttons, Hat, HidMaestro, StandardAxes};
use std::time::Duration;

let mut hm = HidMaestro::spawn()?;
if !hm.is_driver_installed()? {
    hm.install_driver()?; // elevated
}
hm.load_default_profiles()?;

let mut pad = hm.create_controller("xbox-360-wired", None)?;
pad.tap(Buttons::A, Duration::from_millis(80))?;
pad.set_standard_axes(StandardAxes {
    left_stick_x: Some(1.0),
    left_trigger: Some(0.75),
    ..Default::default()
})?;
pad.set_hat(Hat::North)?;
pad.remove()?;
# Ok::<(), hidmaestro::Error>(())
```

Axes are `0.0..=1.0` (`0.5` is stick center and `0.0` is trigger released).
Descriptor-declared axes are also available from `state.axes` by HID usage.

## Troubleshooting

- **Bridge is not found:** run the MCP executable from the bundle directory,
  set `HIDMAESTRO_BRIDGE_PATH` to `hidmaestro-bridge.exe`, or use
  `HidMaestro::builder().bridge_path(...)`.
- **Access denied or install/create fails:** run the bridge elevated. Use the
  named-pipe broker above when the MCP client itself must remain unprivileged.
- **Composite profile fails:** `-composite` profiles such as
  `dualsense-composite` need the usbip-win2 backend; use the
  `install_usbip_backend` tool first.
- **More than four Xbox controllers:** XInput itself exposes at most four
  Xbox-family controllers.

## Bundle contents and unsigned status

The ZIP places `hidmaestro-mcp.exe` and `hidmaestro-bridge.exe` together,
alongside the bridge's self-contained .NET runtime and `HIDMaestro.Core.dll`.
That SDK assembly embeds the HIDMaestro driver, profile catalog, and transport
payload required at runtime. The archive also includes this project's MIT
license, HIDMaestro's license, and `UNSIGNED.txt`.

The bundle is **unsigned**. Verify its SHA-256 file before use. HIDMaestro
creates and trusts its own local certificate when it installs its user-mode
driver; that is separate from signing this distribution archive.

## Source development

Clone with the pinned upstream submodule, then run the normal checks:

```powershell
git clone --recurse-submodules https://github.com/auron-labs/hidmaestro-rs.git
cd hidmaestro-rs
mise install
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo metadata --locked --format-version 1
```

On a Windows x64 machine with .NET 10, Visual Studio Build Tools, and Windows
SDK/WDK 10.0.26100.0, create the bundle with:

```powershell
pwsh -File scripts/package-windows.ps1
```

The script uses `vendor/HIDMaestro` by default. Pass `-HMRepoRoot` or set
`HM_REPO_ROOT` to build against another HIDMaestro checkout. Use `-Mode Stage`
to stage a local bundle without producing a ZIP. See
[development notes](docs/development.md) for hosted-CI limitations and release
ownership.

Ordinary CI does not install the driver. On an elevated Windows x64 machine
with the WDK prerequisites, the manual MCP/XInput smoke validation is:

```powershell
pwsh -File scripts/package-windows.ps1 -Mode Stage
pwsh -File scripts/smoke-windows.ps1
```

This is also available as the manual-only **Windows elevated smoke** workflow;
it is intentionally not run for every push or pull request.

## License

This project is [MIT](LICENSE). HIDMaestro is included as a pinned submodule
and retains its own [license](vendor/HIDMaestro/LICENSE).
