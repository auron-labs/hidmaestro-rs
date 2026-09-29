# MCP operating instructions for agents

[README](../README.md) · For coding tasks, use [AGENTS.md](../AGENTS.md).

## Establish the session

Discover the running server's current tool schemas before operating it. Tool
discovery does not start a bridge or prove a driver works. Call `status` to
connect and inspect `driver_installed` and session controllers. In named-pipe
mode an elevated broker must already be running under the same Windows user;
`start` cannot launch that external broker for the user. If it is missing, give
the user the bridge startup step from the setup guide.

Install the driver only when missing as part of the requested setup. Installation
requires an elevated bridge and sweeps existing HIDMaestro virtual devices.
Do not reinstall it or install a backend reflexively after an input error.
Load profiles, inspect candidates, and choose one with `deployable: true` suited
to the application's input API. `xbox-360-wired` is the simple first choice.
Inspect `requires_usbip_backend`: creating such a profile can install the bundled
backend automatically; use it only when that setup is intended.

Record the controller list before creating a device. Retain the actual key
returned by `create_controller`. Its result is text, not
`structuredContent.key`. If structured data is needed, call `list_controllers`
and read `structuredContent.items`, comparing against the prior list. Do not
assume every successful tool has structured content, that a key is always `c1`,
or that a matching profile ID uniquely identifies your device.

## Send bounded input and observe separately

Wait for the target application to enumerate the controller. Prefer
`tap_buttons` with a short explicit `duration_ms` for a press. Use
`hold_buttons` only when a hold is needed, then explicitly call `release_buttons`.
For ordinary sticks/triggers prefer profile-mapped `set_sticks`. Values are
0–1: stick center is **0.5**, released triggers are **0.0**.

Input persists until changed, released, reset, or the device is removed.
`set_buttons` replaces the pressed set; holds add and releases remove. A tap
releases its named buttons afterward, including ones already held before the
tap. `set_sticks` leaves omitted canonical values unchanged. `set_state` is a
patch, but `standard_axes` replaces the nested object while raw `axes` entries
merge. Cached raw overrides take priority over overlapping canonical values.
Empty objects are not a full reset; use `reset_controller`.

Keep sequences short and include explicit final release/neutral actions.
There is no automatic final reset or rollback of actions already applied.
The server executes calls synchronously: cancellation notifications and the
optional inactivity reset cannot interrupt an active call. The 110,000 ms
limit on requested durations/offsets is an upper bound, not a recommended
operating duration or a guarantee of total completion time.

Observe the target application's input API or visible behavior separately.
Tool success and `get_state` only establish submission/local mirrored state,
not that a game received input. Wait for output events when the application
is expected to send feedback. Empty results alone are inconclusive. Raw events
may have no decoded fields and `crc_valid: false` without CRC failure. Monitor
the cumulative dropped-event count and drain regularly.

## Recover and finish

On failure, report what succeeded and what remains uncertain. Do not claim the
game received input or blindly retry a side-effecting sequence. Attempt reset
and removal of known task-owned keys once the active call returns. A restarted
bridge is a new session: stale keys/state are not recovered automatically,
even if new controllers receive identical key strings.

Normally finish with `reset_controller` and `remove_controller` for each
controller created for this task. Reserve `remove_all_controllers` for
deliberately requested broad cleanup: it affects HIDMaestro devices beyond your
task and can remove driver packages/configuration. When finished with the whole
session, use `shutdown`, then close the MCP connection/stdin. The tool stops the
bridge session but does not itself terminate the MCP input loop. Hard kills do
not guarantee immediate cleanup; a one-connection broker must be restarted for
subsequent use.

Details: [human MCP guide](mcp.md), [state and feedback reference](input-reference.md),
and [troubleshooting](troubleshooting.md).
