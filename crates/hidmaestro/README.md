# hidmaestro

Rust client for [HIDMaestro](https://github.com/hifihedgehog/HIDMaestro) virtual
game controllers. HIDMaestro presents controllers as real Windows hardware to
DirectInput, XInput, SDL3, browser Gamepad, and WGI/GameInput, so this crate is
useful for end-to-end tests that need a controller.

The crate manages a small .NET bridge process (`hidmaestro-bridge`) or connects
to its Windows named pipe and speaks newline-delimited JSON-RPC. The bridge and
driver are Windows x64 only and ship in the
[release bundle](https://github.com/auron-labs/hidmaestro-rs#install-on-windows-x64);
the crate itself compiles everywhere so driver-dependent tests can be
`cfg`-gated.

## Install

```console
cargo add hidmaestro
```

## Example

```rust,no_run
use hidmaestro::{Buttons, Hat, HidMaestro, StandardAxes};
use std::time::Duration;

fn main() -> hidmaestro::Result<()> {
    let mut hm = HidMaestro::spawn()?; // HIDMAESTRO_BRIDGE_PATH, sibling executable, or PATH
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
    Ok(())
}
```

Axes are `0.0..=1.0` (`0.5` is stick center and `0.0` is trigger released).
Descriptor-declared axes are also available from `state.axes` by HID usage.

## License

This project is [MIT](https://github.com/auron-labs/hidmaestro-rs/blob/main/LICENSE).
