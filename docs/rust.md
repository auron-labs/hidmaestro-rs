# Integrate with Rust

[README](../README.md) · [Runtime setup](getting-started.md) · [State reference](input-reference.md)

Add the repository as a Git dependency in your application's `Cargo.toml`.
This pins the API used below; a crates.io publication was not available at the
29 September 2026 check.

```toml
[dependencies]
hidmaestro = { git = "https://github.com/auron-labs/hidmaestro-rs", rev = "44c07e69896b5dc92010e8634afe7006af61dc53" }
```

The dependency supplies the Rust client, not the runtime bridge. Build the
[complete Windows bundle](getting-started.md#build-your-installation) separately
and retain all its files. Actual devices require Windows x64 and an elevated
bridge. The Rust crate itself compiles on other platforms for driver-free tests.

## Create, drive, and remove a controller

Start `hidmaestro-bridge.exe --pipe hidmaestro-mcp` as Administrator, under the
same Windows user as this program. This example performs initial driver setup
only if missing. Installation can remove existing HIDMaestro devices, so run
that setup on a host where this is intended.

Save as `src/main.rs`:

```rust
use hidmaestro::{Buttons, HidMaestro, StandardAxes};
use std::time::Duration;

fn main() -> hidmaestro::Result<()> {
    let mut hm = HidMaestro::builder()
        .pipe_name("hidmaestro-mcp")
        .spawn()?;
    if !hm.is_driver_installed()? {
        hm.install_driver()?;
    }
    hm.load_default_profiles()?;

    let key = hm.create_controller("xbox-360-wired", None)?;
    {
        let mut pad = hm.controller(&key)?;
        pad.tap(Buttons::A, Duration::from_millis(80))?;
        pad.set_standard_axes(StandardAxes {
            left_stick_x: Some(0.75),
            left_trigger: Some(0.25),
            ..Default::default()
        })?;
        std::thread::sleep(Duration::from_millis(100));
        pad.reset()?;
    }
    hm.remove_controller(&key)?;
    hm.shutdown();
    Ok(())
}
```

In a real test, wait for your target to enumerate the controller before the tap
and assert on its input API or visible behavior. The program above demonstrates
submission and cleanup; successful returns do not prove that an application
observed the state. Axes use 0–1, with sticks centered at 0.5 and triggers released
at 0.0.

The `?` paths drop the session on error, requesting bridge shutdown. For a
longer-lived test session, explicitly attempt reset/removal on failure instead
of continuing with uncertain held input. A failed tap release can leave the last
confirmed pressed state active, as the [fake-bridge test](../crates/hidmaestro/tests/fake_bridge.rs)
demonstrates.

## Choose a connection

`HidMaestro::builder().pipe_name("hidmaestro-mcp").spawn()` connects to an
already-running broker. `HidMaestro::spawn()` uses runtime environment settings
and default discovery. For direct spawning, a builder
`.bridge_path(r"C:\tools\hidmaestro\hidmaestro-bridge.exe")` selects an explicit
executable; a configured pipe still takes precedence. The child inherits your
privileges. See [connection order and environment variables](getting-started.md#spawn-the-bridge-directly)
for the canonical rules.

Builder `arg` and `env` apply to a spawned bridge. `response_timeout` sets the
per-RPC deadline (default 120 seconds), and `event_buffer_capacity` sets the
positive output queue capacity (default 1,024). Named-pipe mode is Windows-only
and does not supervise or launch an external broker.

## Submit state and manage borrows

`HidMaestro` owns the session and its controller-state mirror.
`create_controller(profile_id, index)` returns a `String` key. Obtain a
temporary `Controller<'_>` with `hm.controller(&key)?`; it mutably borrows the
session. End that borrow before using session-level operations or a different
controller, then reacquire it when needed.

Dropping the controller view only ends the borrow. It does not unplug the
device. `pad.remove()` consumes the view and removes the device;
`hm.remove_controller(&key)` does the same by key. `hm.shutdown()` consumes the
session. Session drop also requests shutdown. For a spawned child the library
waits up to 30 seconds before terminating it; for an external pipe it sends the
request without waiting for process exit. None of these paths promises immediate
cleanup after a hard kill.

Controller helpers such as `press`, `release`, `set_hat`, `set_axis`, and
`set_standard_axes` clone the mirror and submit immediately, updating it only
after a successful bridge response. `reset` submits `GamepadState::neutral()`.
`submit_state(GamepadState)` replaces the whole state, not selected fields.
The session counterpart takes `hm.submit_state(&key, &state)`.

`hm.state_mut(&key)` only edits the mirror. After ending that borrow, call
`hm.controller(&key)?.submit()?` to push it, or clone the state and use
`hm.submit_state`. These manually edited mirrors are not necessarily submitted
states. `hm.state` and `pad.state` never query the application.

Rust and MCP validation differ. `Controller::set_axis` clamps to 0–1;
`set_standard_axes` replaces its object without MCP's range validation, and
direct `GamepadState` submission does not pass through the MCP patch validator.
Keep values within the [shared ranges and conversion rules](input-reference.md#sticks-triggers-and-raw-axes).
Raw axes still override overlapping canonical values.

## Receive feedback and handle errors

`hm.drain_events()` consumes all queued raw and decoded `OutputEvent`s.
`hm.wait_event(Duration)` returns `Result<Option<OutputEvent>>`, with `None` on
timeout. `wait_event_for_controller(&key, Duration)` filters by key without
discarding unmatched events. Read `dropped_event_count()` to detect cumulative
queue overflow. A reader thread receives events between synchronous RPC calls;
the bounded queue still drops oldest events when full. See
[feedback interpretation](input-reference.md#read-application-feedback).

Operations are synchronous, including `tap` and event waits. Rust durations are
not subject to MCP's 110,000 ms handler limit. A timeout does not undo side
effects or prove that the bridge failed to apply a request; do not blindly
retry device creation or an input sequence.

The public [error enum](../crates/hidmaestro/src/error.rs) distinguishes:

- Discovery/spawn: `BridgeNotFound`, `Spawn`.
- Pipe setup: `InvalidPipeName`, `PipeUnsupported`, `PipeConnect`.
- Transport/serialization: `Io`, `Protocol`, `BridgeExited`, `Timeout`.
- Bridge-reported failures: `Remote` (includes its error text).
- Local lookup/name categories: `NoSuchController`, `UnknownButton`,
  `UnknownAxis`, `UnknownHat`. The `by_name` parsers themselves return `Option`.

Use [troubleshooting](troubleshooting.md) to distinguish privilege, transport,
and profile failures rather than reinstalling the driver reflexively.

## Browse the API locally

From the repository root:

```sh
cargo doc -p hidmaestro --no-deps --open
```

The generated entry point is `target/doc/hidmaestro/index.html`. Source entry
points are [exports](../crates/hidmaestro/src/lib.rs),
[session/controller methods](../crates/hidmaestro/src/client.rs), and
[state/profile/event types](../crates/hidmaestro/src/state.rs).
