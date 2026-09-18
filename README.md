# hidmaestro-rs

Rust crates for [HIDMaestro](https://github.com/hifihedgehog/HIDMaestro) —
virtual game controllers that present as real hardware to every Windows
input API (DirectInput, XInput, SDL3, browser Gamepad, WGI/GameInput).

Built for **end-to-end testing of software that requires a controller**, and
for letting LLM agents drive a virtual gamepad.

## Crates

| Crate | Purpose |
|---|---|
| `hidmaestro` | Client library: start the driver, create controllers, push gamepad state. |
| `hidmaestro-mcp` | MCP stdio server so agents can control a virtual gamepad with tools like `press_buttons` and `set_sticks`. |

## How it works

HIDMaestro's SDK is .NET. This repo includes a small bridge host
(`bridge/HIDMaestro.Bridge`) that wraps `HMContext` and speaks
newline-delimited JSON-RPC over stdio. The `hidmaestro` crate spawns and
drives that process; the MCP server sits on top of the crate.

```
your test / LLM agent
  → hidmaestro (Rust crate)          crates/hidmaestro
  → hidmaestro-bridge.exe (stdio NDJSON-RPC)   bridge/HIDMaestro.Bridge
  → HIDMaestro.Core (.NET SDK) → UMDF2 virtual HID devices
```

## Building the bridge (Windows)

Requires a checkout of the HIDMaestro repo (build it first per its README —
the SDK embeds the driver binaries) and .NET 10:

```bat
git clone https://github.com/hifihedgehog/HIDMaestro
:: inside HIDMaestro: scripts\build_all.cmd
set HM_REPO_ROOT=C:\src\HIDMaestro
dotnet publish bridge\HIDMaestro.Bridge -c Release -r win-x64 --self-contained
```

Then either put `hidmaestro-bridge.exe` on `PATH` or set
`HIDMAESTRO_BRIDGE_PATH` to its full path. Driver install and device
creation require an **elevated** process.

## Using the crate

```rust
use hidmaestro::{Buttons, Hat, HidMaestro, StandardAxes};
use std::time::Duration;

let mut hm = HidMaestro::spawn()?;
if !hm.is_driver_installed()? {
    hm.install_driver()?;                 // elevated
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
```

Axes are `0.0..=1.0` (0.5 = stick center, 0.0 = trigger released). Any
descriptor-declared axis is reachable via `state.axes` keyed by HID usage
(`Axis::X`, `Axis::THROTTLE`, `Axis(0x02C8)`, …); `StandardAxes` covers the
canonical two-sticks-plus-triggers layout and resolves through each
profile's own layout table.

Output reports the game sends *to* the pad (rumble, LEDs, HID PID force
feedback) come back as events: `hm.drain_events()` / `hm.wait_event()`.

## MCP server

```bash
cargo run -p hidmaestro-mcp
```

Register it in any MCP client:

```json
{
  "mcpServers": {
    "hidmaestro": {
      "command": "hidmaestro-mcp",
      "env": { "HIDMAESTRO_BRIDGE_PATH": "C:\\tools\\hidmaestro-bridge.exe" }
    }
  }
}
```

Typical agent flow: `install_driver` → `load_profiles` → `list_profiles` →
`create_controller {profile_id:"xbox-360-wired"}` → `press_buttons`,
`set_sticks`, `set_dpad` → `drain_output_events` to observe rumble/LED.

## Platform notes

- The crates compile everywhere; the bridge and driver only run on Windows
  (elevated). Gate driver-touching test code accordingly.
- `-composite` profiles (e.g. `dualsense-composite`) need the usbip-win2
  backend: `install_usbip_backend`.
- XInput exposes at most 4 Xbox-family controllers at once (XInput's own
  slot limit).

## License

MIT. HIDMaestro itself has its own license — see the upstream repo.
