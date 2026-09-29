# Operate a controller through MCP

[README](../README.md) · [Installation and connection](getting-started.md) · [Input reference](input-reference.md)

## Create your first controller

Start the [elevated bridge and ordinary MCP client](getting-started.md#run-the-bridge-as-administrator)
first. The examples below name a tool and show its **arguments**, not raw
protocol messages. Responses are illustrative, not captured device results.

1. Call **`status`** with `{}`. Its `structuredContent` contains
   `driver_installed` and `controllers`. If the driver is missing and you are
   performing the requested initial setup, call **`install_driver`** with `{}`.
   Installation requires the already-elevated bridge and sweeps existing
   HIDMaestro devices before deployment. Do not repeat it after every error.
2. Call **`load_profiles`** with `{}`, then **`list_profiles`** with
   `{"filter":"xbox"}`. Inspect **`get_profile`** with
   `{"profile_id":"xbox-360-wired"}`. Choose a profile with `deployable: true`;
   inspect `backend`, `requires_usbip_backend`, `button_count`, `axis_count`,
   and `has_hat` rather than inferring support from its name.
3. Call **`create_controller`**:

   ```json
   { "profile_id": "xbox-360-wired" }
   ```

   This also requires elevation. Its result is text, for example:

   ```json
   {
     "content": [
       { "type": "text", "text": "controller 'c1' created from profile 'xbox-360-wired'" }
     ]
   }
   ```

4. Retain the actual key from that result. For structured data, call
   **`list_controllers`** with `{}`. An illustrative `structuredContent` is:

   ```json
   { "items": [{ "key": "c1", "profile_id": "xbox-360-wired" }] }
   ```

   Match the newly created controller against the pre-create list when multiple
   controllers share a profile. Do not assume the first item or the key `c1` is
   yours. Replace `c1` in every following example with your key.
5. Wait for the application to enumerate the controller and open an input
   display or a screen where A has a known effect. Call **`tap_buttons`**:

   ```json
   { "controller": "c1", "buttons": "a", "duration_ms": 80 }
   ```

   On success, A has been pressed and released. Observe the application itself;
   a successful tool result is not proof it received input.
6. Move with **`set_sticks`**:

   ```json
   { "controller": "c1", "left_x": 0.75 }
   ```

   Then recenter with **`set_sticks`**:

   ```json
   { "controller": "c1", "left_x": 0.5 }
   ```

7. Whether observation succeeds or fails, call **`reset_controller`** with
   `{"controller":"c1"}`, then **`remove_controller`** with the same arguments.
   When finished with the session, call **`shutdown`** with `{}` and disconnect
   the MCP server so its stdin closes. The broker must be restarted next time.

Input state persists. `get_state` reports the server's local mirror, not XInput
or the game's state. For application-observed diagnostics, see the
[Windows smoke procedure](development.md#manual-elevated-smoke-validation) and
its host-cleanup warning.

## Send a button combination

To press A and the right bumper together briefly, use **`tap_buttons`**:

```json
{ "controller": "c1", "buttons": ["a", "right_bumper"], "duration_ms": 100 }
```

It returns a text confirmation after submitting both press and release. Other
buttons are unchanged. Named buttons are released afterward even if they were
already held; the tap does not restore a previous snapshot. On failure, attempt
`reset_controller` on your key rather than assuming release succeeded.

## Hold until an explicit release

To hold a modifier while observing the application, call **`hold_buttons`**:

```json
{ "controller": "c1", "buttons": "left_bumper" }
```

It returns immediately after submission; the button stays down. After the
observation, call **`release_buttons`**:

```json
{ "controller": "c1", "buttons": "left_bumper" }
```

The second text confirmation means the release was submitted. Reset the
controller on any interrupted recipe and remove it at the end of the task.
Prefer a tap if there is no reason to keep the hold open between calls.

## Run a short sequence and end neutral

Start with **`reset_controller`** `{"controller":"c1"}` to clear earlier raw
axis overrides. This **`run_action_sequence`** presses A, moves the left stick,
then explicitly releases and recenters:

```json
{
  "controller": "c1",
  "actions": [
    { "at_ms": 0, "press_buttons": "a" },
    {
      "at_ms": 100,
      "release_buttons": "a",
      "state": { "standard_axes": { "left_stick_x": 0.75 } }
    },
    {
      "at_ms": 250,
      "state": {
        "buttons": [],
        "hat": "none",
        "standard_axes": {
          "left_stick_x": 0.5, "left_stick_y": 0.5,
          "right_stick_x": 0.5, "right_stick_y": 0.5,
          "left_trigger": 0.0, "right_trigger": 0.0
        }
      }
    }
  ]
}
```

The result is text confirming completion. The sequence does **not** reset
automatically; its final state remains active. Afterward, or after a failure,
call `reset_controller` on the same key and remove it when finished. An error
may occur after earlier actions have already been applied: do not blindly replay
the sequence. See [sequence semantics](input-reference.md#sequences-and-timing).

## Wait for application feedback

Have the target application trigger rumble, an LED change, or another output
supported by the profile. Then call **`wait_for_output_events`**:

```json
{ "controller": "c1", "timeout_ms": 1000 }
```

It consumes at most one matching queued or newly received event. An illustrative
timeout result's `structuredContent` is:

```json
{ "events": [], "dropped_event_count": 0 }
```

An empty list only means no matching event arrived within that wait. Use
**`drain_output_events`** `{}` to consume all remaining events, including raw
and decoded notifications for the same report. Inspect the
[feedback fields and drop count](input-reference.md#read-application-feedback).
These tools do not change input; reset and remove your controller after the
application check as usual.

## Tool reference

Discover the current declarations with MCP `tools/list`. Discovery works without
starting a bridge; most operational tools connect lazily. The stdio executable
has no interactive command interface, profile-listing flag, or HTTP endpoint.

Below, **text** means `content: [{"type":"text","text":"…"}]` without
`structuredContent`. **JSON** means text containing formatted JSON plus
`structuredContent`: objects appear directly, while arrays appear under
`structuredContent.items`. Tool execution failures return text with
`isError: true`. Omitted optional arguments use the stated defaults.

### Setup tools

| Tool | Arguments | Result and effects |
|---|---|---|
| `start` | None (`{}`) | Text: bridge running/pong. Connects or spawns, and pings. Reconnects after a detected `BridgeExited`; does not recover old keys. In pipe mode it cannot launch a missing external broker. |
| `status` | None | JSON object: `driver_installed` boolean, `controllers` array of key/profile pairs. Connects if needed; does not install a driver. |
| `install_driver` | None | Text. Requires elevation; extracts/signs/installs the HIDMaestro driver. Upstream first sweeps HIDMaestro virtual devices system-wide, even before its same-build installation fast path. Use for requested setup. |
| `install_usbip_backend` | None | Text. Elevated installation of the bundled optional usbip-win2 backend. Choose it only when the requested profile/setup needs it; it is unnecessary for the Xbox 360 walkthrough. |

### Profiles and controllers

| Tool | Arguments | Result and effects |
|---|---|---|
| `load_profiles` | Optional `dir` string; default built-in catalog | Text: number of profiles loaded. A directory is read by the bridge, so use an absolute Windows path for custom profiles. |
| `list_profiles` | Optional `filter` string; default all loaded profiles | JSON array under `items`; case-insensitive substring match on ID, name, or vendor. Does not itself load profiles. |
| `get_profile` | Required `profile_id` string | JSON profile object; unknown ID is a tool error. Metadata includes identity, counts, hat support, backend, deployability, and usbip requirement. It is not an exhaustive capability description. |
| `create_controller` | Required `profile_id`; optional nonnegative integer `index`, default next free upstream index | Text containing the new session key. Elevated device creation; loads defaults if no profiles are loaded. An explicit index must be free and fit the bridge's signed 32-bit integer. It is an upstream enumeration index, not a guaranteed XInput slot. A usbip profile can install its backend automatically on creation in the pinned SDK. |
| `list_controllers` | None | JSON `items` array of `{key, profile_id}` for this bridge session. Supplies structured keys; it is not a system-wide device inventory. |
| `remove_controller` | Required `controller` key | Text. Disposes/hot-unplugs that controller and removes its cached state. Example: `{"controller":"c1"}`. |
| `remove_all_controllers` | None | Text. Disposes session devices, then calls upstream system-wide cleanup, including strays and other HIDMaestro devices. The pinned cleanup also attempts to remove HIDMaestro driver packages/configuration. Requires elevation; reserve for deliberately requested broad cleanup. |

### Input tools

All input tools require a `controller` string containing a key created in the
current session. They submit the resulting state and return text, except
`get_state`. [Button/axis names, ranges, patches, and overrides](input-reference.md)
are shared with the Rust guide.

| Tool | Other arguments | Result and effects |
|---|---|---|
| `tap_buttons` | Required `buttons`, `duration_ms` integer 0–110000 | Adds buttons, waits synchronously, removes those buttons. No default duration. Release can fail if the bridge fails. |
| `hold_buttons` | Required `buttons` | Adds buttons without sleeping; stays held until release/reset. |
| `release_buttons` | Required `buttons` | Removes named buttons, preserving other held buttons. |
| `set_buttons` | Required `buttons` | Replaces the entire pressed set. `{"controller":"c1","buttons":[]}` releases all buttons, leaving axes/hat unchanged. |
| `press_buttons` | Required `buttons`; optional `hold_ms` integer 0–110000 | Legacy behavior: omission holds indefinitely; presence taps, including a zero-duration tap. Prefer explicit hold/tap tools. |
| `set_sticks` | Optional `left_x`, `left_y`, `right_x`, `right_y`, `left_trigger`, `right_trigger`, each 0–1 | Updates only supplied canonical values using profile mapping. Omitted values remain cached; raw overrides remain too. |
| `set_axis` | Required `axis` string and `value` number 0–1 | Writes a raw HID axis override. Example: `{"controller":"c1","axis":"wheel","value":0.5}`; use only for a profile exposing that usage, and reset afterward. |
| `set_dpad` | Required `direction` string | Sets discrete hat, clears continuous angle. `{"controller":"c1","direction":"none"}` releases it. |
| `set_state` | Required `state` patch object | Applies a validated patch and submits once. Nested canonical axes replace; raw axes merge. Example: `{"controller":"c1","state":{"buttons":[],"hat":"none"}}`. The handler tolerates omitted `state` as an empty patch, but discovery declares it required: supply it. |
| `get_state` | No other arguments | JSON mirrored `GamepadState`: integer `buttons`/`hat`, optional axes and other fields. Raw axis map keys serialize as decimal usage strings. Not an application read-back. |
| `run_action_sequence` | Required `actions` array | Text after synchronous execution. 1–128 actions, each with required `at_ms` (0–110000, non-decreasing) and at least one of `state`, `press_buttons`, `release_buttons`, `dpad`, `axes`. No implicit final reset. See the recipe and [ordering rules](input-reference.md#sequences-and-timing). |

### Reset controller

**`reset_controller`** takes required `controller` and no other arguments:
`{"controller":"c1"}`. It submits a full neutral state, clearing buttons,
hat, canonical values, raw overrides, and optional fields; upstream supplies
neutral axis defaults. It returns text and leaves the device connected. Use
`remove_controller` afterward to unplug it.

### Feedback tools

| Tool | Arguments | Result and effects |
|---|---|---|
| `drain_output_events` | None | JSON object `{events, dropped_event_count}`; consumes all buffered events from all session controllers. |
| `wait_for_output_events` | Required `timeout_ms` integer 0–110000; optional `controller` string, default any | Same object shape with zero or one event. Consumes the first match; preserves unmatched events subject to the bounded queue. A zero timeout checks the queue immediately. No controller existence check is performed for the filter. |

### Shutdown tool

**`shutdown`** takes `{}`. It drops the bridge session and returns text
(`bridge stopped; all virtual controllers removed`). That message describes the
requested session cleanup, not verified system-wide removal. It does not invoke
the broad `remove_all_controllers` operation and does not stop the MCP input
loop. Later bridge-dependent tools can start a new session; a pipe-mode broker
must first be restarted externally.

## Protocol and process lifetime

MCP uses JSON-RPC 2.0 messages, one JSON object per line on stdio. For example,
after legacy initialization, the tap above is framed as this **`tools/call`
request**, sent on one line:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tap_buttons","arguments":{"controller":"c1","buttons":"a","duration_ms":80}}}
```

The JSON-RPC response wraps the tool result under `result` and echoes `id`.
The .NET bridge's internal NDJSON request/response format is a different
protocol; do not send MCP envelopes to the bridge or its named pipe.

The implementation and [stdio tests](../crates/hidmaestro-mcp/tests/mcp_stdio.rs)
support these flows:

- **Legacy:** `initialize` with `params.protocolVersion: "2025-03-26"`
  (also the default when omitted), followed by `notifications/initialized` and
  tools requests. Other versions in `initialize` are rejected with `-32602`.
- **Modern:** `server/discover` with per-request metadata as below. Discovery
  returns `supportedVersions: ["2026-07-28"]` and tools capabilities with
  `listChanged: false`. After discovery, each request requires this metadata.
  Metadata can also select modern response formatting on an individual request
  without discovery.

```json
{
  "jsonrpc": "2.0",
  "id": "discover",
  "method": "server/discover",
  "params": {
    "_meta": {
      "io.modelcontextprotocol/protocolVersion": "2026-07-28",
      "io.modelcontextprotocol/clientCapabilities": {}
    }
  }
}
```

Modern results include `resultType: "complete"` and
`_meta["io.modelcontextprotocol/serverInfo"]`. Missing/malformed metadata yields
`-32602`; an unsupported modern version yields `-32022` with
`data.supported` and `data.requested`. These are implemented variants, not a
claim of compatibility with every MCP client.

Invalid JSON yields `-32700`; invalid requests use `-32600`, unknown methods
`-32601`, and malformed `tools/call` envelopes or unknown tool names `-32602`.
A recognized tool whose handler fails instead returns a normal JSON-RPC
`result` with `isError: true`. Handler validation is not a generic JSON Schema
validator; follow discovery's declarations even where the handler is permissive.

`notifications/initialized` and `notifications/cancelled` receive no reply.
Cancellation does not interrupt an active call. Unknown notifications receive
no reply either. Use requests with IDs for tool calls so you can observe errors.
MCP `ping` returns an empty result without checking the bridge; the `start` tool
does check it.

Calls run synchronously. While a tap, wait, sequence, or driver operation is
active, later calls and the inactivity reset cannot execute. Short sleep slices
do not make those calls interruptible. Stdin EOF ends the MCP input loop and
requests bridge shutdown after pending work; the `shutdown` tool alone leaves
the loop open. Graceful shutdown gives a spawned bridge time to dispose devices.
Hard kills and failed connections do not guarantee immediate release or cleanup.
