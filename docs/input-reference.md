# Input, state, and feedback

[README](../README.md) · [MCP guide](mcp.md) · [Rust guide](rust.md)

## Choose a profile and keep its controller key

A **profile** describes a device model: identity, input layout, backend, and
deployment requirements. A **controller** is a live virtual device created from
that profile. `xbox-360-wired` is a profile ID; a returned key such as `c1`
identifies one controller in the current bridge session. Keep the actual key,
not a guessed ordinal. A restarted bridge is a new session even if it reuses
the same key strings.

Load profiles and inspect their metadata before creating a device. Returned
metadata includes `id`, `name`, `vendor`, `vendor_id`, `product_id`,
`display_name`, `connection`, `backend`, `button_count`, `axis_count`, `has_hat`,
`deployable`, and `requires_usbip_backend`. It does not expose every layout or
feedback capability. Not every profile supports every button, axis, sensor,
battery field, or output feature. Choose a deployable profile suitable for the
application; the first walkthrough uses `xbox-360-wired`.

**Input** travels from your code/tools to the virtual device for the application
to read. **Output feedback** travels from the application to the virtual device
(for example, rumble or LEDs) and is forwarded as events. Neither a successful
submission nor a local cached state proves that the application read the input.

## Buttons

MCP accepts a single name such as `"a"`, an array such as `["a","start"]`, or
an unsigned integer mask. Prefer names. Names are case-insensitive, with spaces
and hyphens converted to underscores; they are not otherwise trimmed. These
aliases come from [the parser](../crates/hidmaestro/src/state.rs):

| Name | Additional accepted names | Mask bit |
|---|---|---|
| `a` | `cross` | 0 |
| `b` | `circle` | 1 |
| `x` | `square` | 2 |
| `y` | `triangle` | 3 |
| `left_bumper` | `lb`, `l1` | 4 |
| `right_bumper` | `rb`, `r1` | 5 |
| `back` | `select`, `view`, `share_x360` | 6 |
| `start` | `options`, `menu` | 7 |
| `left_stick` | `l3` | 8 |
| `right_stick` | `r3` | 9 |
| `guide` | `home`, `ps`, `xbox` | 10 |
| `touchpad` | — | 11 |
| `share` | — | 12 |
| `right_paddle` | — | 13 |
| `left_paddle` | — | 14 |
| `misc1` | `mic`, `c` | 15 |
| `right_paddle2` | — | 16 |
| `left_paddle2` | — | 17 |

For advanced mask input, each button is `1 << bit`; A plus Start is 129.
MCP rejects negative masks, values outside `u32`, and unknown bits. `[]` or `0`
means no buttons; `"none"` is not a button alias. `share` and `back` are distinct.

`set_buttons` replaces the whole pressed set. Hold/press adds buttons, and
release removes them. A tap adds the named buttons and then removes them; it
does not restore buttons that were already held before the tap. If release
submission fails, input may remain held. Reset your controller explicitly.

## D-pad and continuous hats

The name normalization is the same as for buttons. MCP direction/patch inputs
use strings; integer values below describe Rust/wire serialization and
`get_state`, not accepted MCP direction arguments.

| Direction | Accepted names | Serialized value |
|---|---|---|
| Released | `none`, `neutral`, `released`, `center`, `centre` | 0 |
| Up | `north`, `up`, `n` | 1 |
| Up-right | `north_east`, `up_right`, `ne` | 2 |
| Right | `east`, `right`, `e` | 3 |
| Down-right | `south_east`, `down_right`, `se` | 4 |
| Down | `south`, `down`, `s` | 5 |
| Down-left | `south_west`, `down_left`, `sw` | 6 |
| Left | `west`, `left`, `w` | 7 |
| Up-left | `north_west`, `up_left`, `nw` | 8 |

`hat_degrees` is a continuous angle: 0° is north, increasing clockwise. MCP
requires a finite value in `[0, 360)`. An explicit angle takes priority over
the discrete hat during conversion, subject to the profile's resolution.
`set_dpad`, a patch's `hat`, Rust `set_hat`, or a sequence's `dpad` clears a
stored continuous angle. In one patch, `hat` is applied before `hat_degrees`,
so an explicit numeric angle can override that discrete update.

## Sticks, triggers, and raw axes

All axis values use **`0.0..=1.0`**, not `-1..1`. Stick center is **`0.5`**;
released triggers are **`0.0`**. Prefer profile-mapped canonical axes:

| MCP `set_sticks` argument | `standard_axes` / Rust `StandardAxes` field | Neutral |
|---|---|---|
| `left_x`, `left_y` | `left_stick_x`, `left_stick_y` | 0.5 |
| `right_x`, `right_y` | `right_stick_x`, `right_stick_y` | 0.5 |
| `left_trigger`, `right_trigger` | Same names | 0.0 |

`set_sticks` updates only supplied canonical values. In bridge conversion,
unspecified fields of the resulting `standard_axes` object use these neutral
defaults, and the profile determines which HID axes receive them.

Raw axis names are for unusual or descriptor-specific controls. Their aliases
are fixed HID usages, **not** profile-aware stick mapping:

| Raw name | Aliases | Usage |
|---|---|---|
| `x` | `left_x`, `left_stick_x` | `0x0130` |
| `y` | `left_y`, `left_stick_y` | `0x0131` |
| `z` | `left_trigger`, `lt` | `0x0132` |
| `rx` | `right_x`, `right_stick_x` | `0x0133` |
| `ry` | `right_y`, `right_stick_y` | `0x0134` |
| `rz` | `right_trigger`, `rt` | `0x0135` |
| `slider`, `dial`, `wheel` | — | `0x0136`, `0x0137`, `0x0138` |
| `rudder`, `throttle` | — | `0x02BA`, `0x02BB` |
| `accelerator` | `gas` | `0x02C4` |
| `brake`, `clutch`, `steering` | — | `0x02C5`, `0x02C6`, `0x02C8` |

Advanced axis keys accept hexadecimal strings such as `"0x0130"` or decimal
strings such as `"304"` (both X), within `u16`. Rust also exposes constants
`Axis::VX`/`VY`; the name parser does not accept `"vx"`/`"vy"`, so use their
usage values `"0x0140"`/`"0x0141"` if needed. A parseable usage does not mean
the profile implements it. `get_state` serializes raw keys as decimal strings.

The bridge applies raw `axes` **after** canonical axes, overriding overlapping
values. A previous raw override stays in the cached state across `set_sticks`
calls. If recentering appears ineffective, reset before using canonical axes.

## Patch state without surprising replacements

MCP `set_state` applies a patch to the local mirror, validates it, and submits
the resulting full state. Unknown top-level patch fields are rejected. Omitted
fields remain unchanged, except that supplying `hat` clears `hat_degrees`.

| Field | Supplied value | Omitted / explicit `null` |
|---|---|---|
| `buttons` | Replaces pressed set | Both leave unchanged |
| `hat` | Sets discrete direction and clears angle | Both leave unchanged |
| `axes` | Merges supplied raw entries | Both leave unchanged; `{}` also leaves existing entries |
| `standard_axes` | Replaces the entire canonical object | Omitted preserves; `null` clears the object |
| `hat_degrees` | Sets continuous angle | Omitted preserves; `null` clears angle |
| `battery_level` | Integer 0–10, not a percentage | Omitted preserves; `null` clears |
| `battery_charging` | Boolean | Omitted preserves; `null` clears |
| `accel_g` | Three finite values in g, +X right, +Y up, +Z toward player | Omitted preserves; `null` clears |
| `gyro_dps` | Three finite values in degrees/second, right-hand rule around the same axes | Omitted preserves; `null` clears |

The handler treats `null` for `buttons`, `hat`, and `axes` as omission, but their
tool schema does not permit `null`; use omission in client requests. Nullable
optional fields are not a promise of sensor/battery support in every profile.
Clearing removes an explicit override; the bridge constructs a fresh upstream
state with defaults for absent fields.

For example, after these two **state patch objects** in order:

```json
{
  "standard_axes": { "left_stick_x": 0.8, "left_trigger": 0.6 },
  "axes": { "x": 0.9 }
}
```

```json
{
  "standard_axes": { "right_stick_x": 0.7 },
  "axes": { "y": 0.2 }
}
```

The cached canonical object contains only `right_stick_x: 0.7`; left X and
left trigger now use canonical defaults on conversion. The raw map still
contains **both** X=0.9 and Y=0.2 and overrides any overlapping canonical axes.
Thus an empty object is not a universal reset: `standard_axes: {}` replaces
canonical values, but `axes: {}` does not clear raw values. Use
[`reset_controller`](mcp.md#reset-controller) for a full neutral reset.

Rust `submit_state` submits a complete `GamepadState`, replacing the mirror on
success; it does not apply MCP patch rules. `Controller::set_standard_axes`
replaces its canonical object, and `Controller::set_axis` clamps to 0–1, whereas
MCP rejects out-of-range axis values. Other Rust setters/full submissions do
not share all MCP validation. Direct `state_mut` edits change only the mirror
until explicitly submitted. See [Rust state usage](rust.md#submit-state-and-manage-borrows).

## Sequences and timing

MCP sequences contain 1–128 actions. Every action has a nonnegative integer
`at_ms`, relative to the start of execution, at most 110,000 milliseconds.
Offsets must be non-decreasing; equal offsets execute in array order. Each
action must include a state change.

Within an action, order is: `state` patch, `press_buttons`, `release_buttons`,
`dpad`, then raw `axes`. The resulting full state is submitted once per action.
Actions are validated before execution, but a later bridge error does not roll
back earlier successful submissions. The final state stays active: include
explicit releases/neutral values and reset after the sequence.

Execution is synchronous. Offsets are scheduling targets, not frame-perfect or
hard real-time guarantees; RPC and scheduling delays can make an action late.
The 110,000 ms bound also applies to MCP taps, legacy timed presses, and feedback
waits. It limits requested waits/offsets, not total wall-clock time including
bridge calls. Cancellation and inactivity handling cannot interrupt active calls.

## Read application feedback

Events have these fields:

| Field | Meaning |
|---|---|
| `controller` | Session key of the controller receiving output |
| `report_id` | Report identifier byte |
| `fields` | Upstream-decoded JSON fields, or an empty object for raw output |
| `raw` | Report bytes as an integer array; bridge raw events prepend the report ID |
| `crc_valid` | Decoded CRC result, or `false` for raw/unverified events |

One application report can produce a raw notification and a second decoded
notification. Raw `crc_valid: false` means it was not checked, not that its CRC
failed. Decoded field names depend on upstream's profile/report decoder; inspect
actual events rather than assuming a universal rumble or LED field.

The Rust client queues raw and decoded events between calls. The default
capacity is **1,024 events**. When full it drops the oldest and increments a
cumulative session `dropped_event_count`; draining does not reset this counter.
Rust builders can choose a positive `event_buffer_capacity`; MCP uses the
default. A filtered wait consumes one matching event and leaves unmatched
events queued, still subject to the same drop policy. Delivery is not lossless.

An empty drain or timed-out wait alone does not imply input failure: the
application may send no output, or the profile/API may not produce the feedback
you expected. Observe application input separately and use the
[feedback troubleshooting checks](troubleshooting.md#no-feedback-or-an-increasing-drop-count).
