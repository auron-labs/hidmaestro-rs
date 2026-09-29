# Working on hidmaestro-rs

hidmaestro-rs creates virtual game controllers on Windows through the pinned
HIDMaestro implementation. This repository supplies a Rust client, a stdio MCP
server, and a .NET bridge. Rust-only development/tests can run off Windows;
actual controllers require the Windows x64 bridge and native payload.

## Read the owning guide

- [README](README.md): purpose and first use.
- [Development](docs/development.md): contributor setup, internals, tests,
  packaging, CI, and release ownership.
- [Setup](docs/getting-started.md): prerequisites, discovery, named pipes,
  runtime environment variables.
- [MCP](docs/mcp.md), [Rust](docs/rust.md), and
  [input reference](docs/input-reference.md): human-facing contracts.
- Runtime agents operating tools should use [agent usage](docs/agent-usage.md).

## Locate implementation and tests

| Area | Sources |
|---|---|
| Rust session, transport, cache, event buffer, controller views | `crates/hidmaestro/src/client.rs` |
| State types, serialization, input aliases | `crates/hidmaestro/src/state.rs`, `tests.rs` |
| Public exports/errors, bridge wire types | `crates/hidmaestro/src/{lib,error,protocol}.rs` |
| MCP schemas/handlers, patches, sequences, negotiation, main loop | `crates/hidmaestro-mcp/src/main.rs` |
| Bridge dispatch, pipe security/lifetime, state conversion, events | `bridge/HIDMaestro.Bridge/Program.cs` |
| Process-level client/lifetime tests | `crates/hidmaestro/tests/{fake_bridge,graceful_shutdown}.rs` |
| Real MCP stdio with driver-free stand-ins | `crates/hidmaestro-mcp/tests/mcp_stdio.rs` |
| Native SDK, driver, profiles | Pinned `vendor/HIDMaestro` submodule |
| Bundles and native observation | `scripts/package-windows.ps1`, `scripts/smoke-windows.ps1` |

## Check changes

Use the repository tool configuration (`mise.toml`, `hk.pkl`). From the root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo metadata --locked --format-version 1
```

`mise exec --` selects configured tools when needed. Rust is pinned to 1.98.1;
CI uses stable. The declared 1.75 MSRV is not a tested CI target.
Library process tests need Python 3 via `python3` or `python`; MCP fake launchers
use `python3` on Unix and `py -3` on Windows. Unset `HIDMAESTRO_PIPE_NAME` for
fake-bridge tests so pipe configuration does not override the fake executable.

These checks require no real bridge, driver, .NET SDK, or WDK. Full packaging
requires Windows x64, .NET 10, PowerShell, Visual Studio C++ x64/ARM64 tools,
and SDK/WDK 10.0.26100.0. Native smoke additionally requires elevation and
permission to mutate the host. Ordinary CI does not validate a native build
or application-observed input. Report missing infrastructure and actual check
results precisely; continue independent work that can be completed.

For changed runnable Markdown Rust examples, compile-check in a temporary crate
outside the repository with a path dependency; do not execute device creation
just to validate a snippet. JSON examples should agree with schemas and handlers.

## Preserve implementation contracts

- Keep MCP stdout protocol-only. The bridge's internal NDJSON and MCP's JSON-RPC
  are distinct protocols; bridge-only operations are not automatically public tools.
- Keep MCP tool declarations and handlers consistent. For input/protocol changes,
  review the corresponding unit/process tests and `docs/mcp.md`/
  `docs/input-reference.md`; update other guides only where their usage changes.
- Keep mirror updates and bridge conversion explicit: successful submission is
  not application observation, raw axes override canonical axes, and patches
  are not universally recursive merges. Review `client.rs`, `state.rs`, and
  `Program.cs` together when changing these behaviors.
- Preserve synchronous call/lifetime semantics in documentation. Do not promise
  cancellation, implicit sequence reset, lossless events, or immediate hard-kill
  cleanup unless the implementation establishes them.
- Use the pinned upstream dependency. Do not replace or update the submodule
  incidentally; integration changes should be intentional and in task scope.
- Do not fabricate compatibility, release availability, crate publication,
  native-test success, or Windows smoke results. Inspect current code and actual
  artifacts rather than trusting prose, comments, or workflow presence alone.

## Keep work scoped

Respect the requested task and preserve existing user changes. Do not install
drivers, run broad device cleanup, or trigger the host-mutating smoke workflow
for an ordinary coding/documentation task. `remove_all_controllers` reaches
system-wide upstream cleanup, including driver package/configuration removal;
installation also sweeps devices. Use a dedicated host for intentional native work.

Follow existing repository patterns; avoid adding a documentation framework or
extra approval/checkpoint machinery for routine work. Release Please owns
versions, tags, changelogs, and releases. Do not publish or bump versions as an
incidental part of a coding task.
