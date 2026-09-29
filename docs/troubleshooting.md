# Troubleshoot a controller session

[README](../README.md) · [Setup](getting-started.md) · [MCP operations](mcp.md)

## No published download or latest-release page

**Likely cause:** no release bundle has been published for this repository (as
of 29 September 2026). **First check:** the
[releases list](https://github.com/auron-labs/hidmaestro-rs/releases), not the
manifest version or presence of a workflow.

**Fix:** follow the [source-build route](getting-started.md#build-your-installation).
Upstream's releases are for HIDMaestro, not this project's bridge/MCP bundle.

## The MCP executable appears to do nothing

**Likely cause:** it is waiting for newline-delimited MCP messages on stdin.
It is not an interactive CLI. **First check:** confirm your MCP client launches
it as a stdio server and performs tool discovery.

**Fix:** use the [launch configuration shape](getting-started.md#connect-an-ordinary-mcp-client).
Do not type shell commands into its input. Discovery can succeed without a
bridge; follow with `status` to check the actual bridge/driver connection.

## Bridge executable or bundle files not found

**Likely cause:** only Rust was built, the bundle was split, or discovery points
to the wrong executable. **First check:** the configured
`HIDMAESTRO_BRIDGE_PATH` and the directory containing the running MCP executable.

**Fix:** stage the [complete bundle](getting-started.md#build-your-installation)
and keep its runtime/SDK files together. Direct spawning looks beside the
**current executable**, not in the shell's current directory, before searching
`PATH`. Set an absolute bridge path if needed. A Rust test executable under
`target` does not automatically discover a bridge in your repository root.
If a pipe is configured, it takes precedence over all executable discovery.

## Access denied during installation or controller creation

**Likely cause:** the bridge is not elevated. **First check:** which terminal
started the bridge, not merely whether the MCP client was elevated.

**Fix:** start the [bridge as Administrator](getting-started.md#run-the-bridge-as-administrator)
and reconnect from an ordinary client under the same Windows user. Directly
spawned bridges inherit the parent process's privileges. If an already-elevated
bridge reports an installation failure, inspect its stderr and returned error
before taking further action; do not reinstall every driver or change Windows
signing policy as a first response.

## Named-pipe connection fails

**Likely cause:** the broker is absent, has already served its one connection,
is running under another Windows account, or the name is invalid.
**First check:** the broker terminal should say it is waiting for one client,
and its name must match `HIDMAESTRO_PIPE_NAME` exactly.

**Fix:** start a new elevated broker before reconnecting. Use the same Windows
user at both ends. Supply a 1–128-character ASCII name starting with a letter
or digit, then only letters, digits, `-`, `_`, `.`; omit paths and the
`\\.\pipe\` prefix. The broker exits after disconnect/shutdown and is not a
multi-client service. `start` cannot launch an external broker in pipe mode.
Named pipes are unsupported off Windows; unset that configuration for
driver-free development. See [connection details](getting-started.md#run-the-bridge-as-administrator).

## Profiles are absent or the ID is unknown

**Likely cause:** the catalog was not loaded, a custom directory was selected,
or the supplied string is not a profile ID. **First check:** call `load_profiles`
with `{}`, then `list_profiles` with an appropriate substring filter.

**Fix:** use the exact `id` returned by that catalog. Inspect it with
`get_profile`; use the separately returned controller key for input operations.
Custom `dir` paths are resolved by the bridge; use an absolute Windows path.
See [profile concepts](input-reference.md#choose-a-profile-and-keep-its-controller-key).

## A profile cannot be deployed or needs usbip

**Likely cause:** `deployable` is false (upstream rejects profiles without a
deployable descriptor), or the selected backend has additional requirements.
**First check:** `get_profile` and the complete tool error, especially
`backend` and `requires_usbip_backend`.

**Fix:** choose a deployable profile suited to the task; `xbox-360-wired` avoids
the optional composite backend for a first test. For a deliberately selected
usbip profile, run the bridge elevated. The pinned upstream installs its bundled
backend on first create; `install_usbip_backend` can perform that setup in
advance. Do not infer backend requirements solely from an ID suffix or install
it reflexively after unrelated errors. [Tool reference](mcp.md#profiles-and-controllers).

## Controller exists but the application does not see input

**Likely cause:** enumeration is incomplete, the application uses a different
input API/profile, or the observation is only the cached state.
**First check:** confirm the application enumerates the newly created device
through its own input API or input display. `get_state` is not that check.

**Fix:** allow enumeration before input, select the intended controller in the
application, and test a short press with an explicit release. For Xbox-family
profiles, XInput's four slots can be occupied; the pinned upstream documents this
limit and the smoke script checks slots 0–3. Free a slot before a dedicated
XInput test. See the [manual Windows smoke procedure](development.md#manual-elevated-smoke-validation)
for an external check of enumeration and A press/release. It mutates the host
and broadly removes HIDMaestro devices; do not run it casually on a shared host.
A passing smoke check does not establish every API/profile or game's behavior.

## Buttons stay held or a stick will not recenter

**Likely cause:** persistent input, a failed release, a raw override, or a
misunderstood nested patch. **First check:** inspect `get_state` for the
task-owned key, including both `axes` and `standard_axes`.

**Fix:** call `reset_controller` with that key, then use short `tap_buttons`
operations and profile-mapped `set_sticks`. A hold needs `release_buttons`;
a sequence needs explicit final release/neutral actions. A tap removes named
buttons afterward rather than restoring a preexisting hold. Raw axes override
canonical axes and survive ordinary `set_sticks` calls. `standard_axes` in a
patch replaces the nested object, but raw `axes` merge; `{}` is not a universal
reset. See [state patch rules](input-reference.md#patch-state-without-surprising-replacements).

## A long call prevents release or shutdown

**Likely cause:** the server executes calls synchronously. **First check:** the
active tap duration, wait timeout, or sequence's final offset.

**Fix:** let the active call return, then reset/remove your controller. Use short
durations on subsequent calls. Cancellation notifications and inactivity reset
cannot interrupt an active operation, and later release/shutdown calls queue
behind it. After an error, inspect state and the application before retrying;
already-applied actions are not rolled back. Hard-killing a process does not
promise immediate device cleanup. [Timing semantics](input-reference.md#sequences-and-timing).

## A controller key stopped working after reconnecting

**Likely cause:** a restarted bridge has a new session and a fresh controller
map. **First check:** call `status`/`list_controllers` and compare with the keys
you retained.

**Fix:** set up the new session, create a controller, and retain its new key.
`start` can recover a directly spawned bridge after a detected exit, but does
not restore controllers or cached state. Old strings may be reused for new
devices; do not assume they identify your previous device. Use
`reset_controller`/`remove_controller` for known task-owned devices, reserving
`remove_all_controllers` for intentionally requested system-wide cleanup.

## No feedback or an increasing drop count

**Likely cause:** the application is not sending output, the profile/API does
not supply the expected feedback, the filter key is wrong, or events arrive
faster than they are consumed. **First check:** trigger a known application
output and drain events, checking `controller`, `report_id`, `raw`, and
`dropped_event_count`.

**Fix:** use a short `wait_for_output_events` for the actual key and drain
regularly. Rust callers can increase `event_buffer_capacity`. The counter is
cumulative and does not reset on drain. Raw events can have empty decoded
fields and `crc_valid: false` without a CRC failure; one report may produce raw
and decoded events. An empty result alone does not establish input failure.
[Feedback reference](input-reference.md#read-application-feedback).

## Source build lacks the submodule or native payload

**Likely cause:** an incomplete clone, missing native tools, or skipping the
upstream build. **First check:** `git submodule status` and the first failing
command in packaging output.

**Fix:** from the repository root, run `git submodule update --init --recursive`,
install the [native prerequisites](getting-started.md#requirements), and rerun
`pwsh -File scripts/package-windows.ps1 -Mode Stage` without
`-SkipUpstreamBuild`. Both x64 and ARM64 payloads are needed. `dotnet restore`
alone does not build the native resources; one isolated SDK build on a clean
tree can lack embedded payloads. The package script calls upstream's two-pass
SDK build. [Packaging details](development.md#windows-bundle).

## MCP protocol negotiation fails

**Likely cause:** an unsupported version or missing modern per-request metadata.
**First check:** the response's JSON-RPC `error.code` and requested version.

**Fix:** use a client that can negotiate one of the
[implemented flows](mcp.md#protocol-and-process-lifetime): legacy `initialize`
with `2025-03-26`, or modern discovery/metadata with `2026-07-28`.
`-32602` can mean malformed parameters; `-32022` reports an unsupported modern
version and includes supported versions. A tool `result.isError` is a different
failure path: inspect its text for a bridge or input error. Do not send the
bridge's internal NDJSON format to the MCP executable.
