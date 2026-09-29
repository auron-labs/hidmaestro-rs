# Development and releases

[README](../README.md) · [First-time Windows installation](getting-started.md) · [Repository agent instructions](../AGENTS.md)

## Prerequisites

For Rust-only work, install Rust with rustfmt/Clippy and Python 3. The repository
pins Rust 1.98.1 through [mise.toml](../mise.toml); hosted CI uses rolling
`stable`, not the manifest's declared 1.75 MSRV. `mise install` provisions the
declared tools, including .NET 10 and the repository linters. .NET and native
Windows tools are unnecessary for Rust-only checks.

For full bridge/native work, use Windows x64 with .NET SDK 10, PowerShell 7,
Windows PowerShell, Visual Studio 2022+ C++ x64 **and ARM64** tools, and Windows
SDK/WDK 10.0.26100.0 (UMDF 2.15). See
[installation prerequisites](getting-started.md#requirements). Elevation is
needed to install drivers/create devices, not simply to edit or check Rust.

From a source parent directory:

```sh
git clone --recurse-submodules https://github.com/auron-labs/hidmaestro-rs.git
cd hidmaestro-rs
```

Use `git submodule update --init --recursive` from an existing clone to recover
missing upstream sources. `vendor/HIDMaestro` is pinned to
`942e25aa9ce4e93dc601f59a796f7432809fc41b`; ordinary development should use it.

## Find the behavior to change

| Source | Responsibility |
|---|---|
| [`crates/hidmaestro/src/client.rs`](../crates/hidmaestro/src/client.rs) | Bridge discovery/transports, synchronous RPC, session lifetime, mirrored state, event queue, borrowed controller views |
| [`crates/hidmaestro/src/state.rs`](../crates/hidmaestro/src/state.rs) | Public state/profile/event types, button/hat/axis parsers and serialization |
| [`protocol.rs`](../crates/hidmaestro/src/protocol.rs), [`error.rs`](../crates/hidmaestro/src/error.rs) | Internal bridge wire types and public errors |
| [`crates/hidmaestro-mcp/src/main.rs`](../crates/hidmaestro-mcp/src/main.rs) | MCP discovery/schemas, handlers, patch validation, sequences, protocol negotiation, stdio loop |
| [`bridge/HIDMaestro.Bridge/Program.cs`](../bridge/HIDMaestro.Bridge/Program.cs) | .NET SDK calls, pipe broker, session keys, state conversion and output event forwarding |
| [`vendor/HIDMaestro`](../vendor/HIDMaestro) | Pinned upstream SDK, profiles, native driver/backend implementation |
| [`scripts`](../scripts), [workflows](../.github/workflows) | Bundle assembly, manual native smoke, hosted checks and release ownership |

```text
MCP client -- JSON-RPC 2.0 / stdio --> hidmaestro-mcp
                                           |
Rust application --------------------> hidmaestro Rust client
                                           |
                          internal NDJSON / child stdio OR local named pipe
                                           |
                                  .NET hidmaestro-bridge
                                           |
                                  upstream HIDMaestro SDK / driver
                                           |
                                     virtual controller <--> application
```

Rust supplies the test/application API and MCP process. .NET hosts the existing
upstream SDK and driver payload instead of reimplementing it. The bridge's
internal messages use `id`, `method`, `params` and `ok`/`result`/`error`, with
interleaved event lines. They are not MCP JSON-RPC envelopes. Bridge-only
operations such as raw-report submission and name finalization are not public
Rust methods or MCP tools.

Change MCP argument validation and tool declarations together. Follow input
changes through the Rust mirror and `BuildState`: canonical axes map through
the profile, then raw axes override. Keep documentation's distinction between
submitted/cached state and application-observed input. Transport, lifetime, and
event changes belong with the client tests, not only tool schemas.

## Run Rust-only checks

From the repository root, with the configured Rust toolchain on `PATH` (or
prefix commands with `mise exec --`):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo metadata --locked --format-version 1
```

These check formatting, compile/lint all Rust targets, run unit/process/rustdoc
tests, and resolve locked workspace metadata. They do not compile the native
driver, install it, or establish hardware-dependent behavior. Rust doc tests
compile crate examples; they do not automatically compile Markdown code fences.
Compile-check complete documentation examples separately without running them
against a real device.

Python launcher requirements matter for the process tests:

- Library [`fake_bridge.rs`](../crates/hidmaestro/tests/fake_bridge.rs) and
  [`graceful_shutdown.rs`](../crates/hidmaestro/tests/graceful_shutdown.rs) search
  `python3`, then `python`. The selected interpreter must be Python 3.
- MCP [`mcp_stdio.rs`](../crates/hidmaestro-mcp/tests/mcp_stdio.rs) uses
  `/usr/bin/env python3` on Unix and `py -3` on Windows for its fake launcher.
- Keep `HIDMAESTRO_PIPE_NAME` unset in these tests: a configured pipe overrides
  their explicit fake executable. The tests do not need an elevated process.

Unit tests cover serialization/parsers, patch behavior, sequences, protocol
responses, pipe validation, and bounded events. Fake-bridge process tests cover
RPC/event plumbing, release failure state, graceful disposal, MCP discovery,
EOF, and reconnection. They are client/protocol tests using Python stand-ins,
not native driver validation. To exercise just MCP stdio coverage:

```sh
cargo test -p hidmaestro-mcp --test mcp_stdio --locked
```

`mise` tasks also expose `fmt`, `check`, `clippy`, and `test`.
[`hk.pkl`](../hk.pkl) configures pre-commit/check/fix steps including cargo
checks, dependency/license checks, leak checks, filename checks, and text
hygiene; commit messages use Conventional Commits. Prefer check mode when
validating a scoped change so auto-fix hooks do not rewrite unrelated work.
A Windows smoke run is not required for ordinary documentation or Rust-only work.

## Windows bundle

From the repository root on a provisioned Windows machine:

```powershell
pwsh -File scripts/package-windows.ps1
```

The [packaging script](../scripts/package-windows.ps1) calls upstream
`scripts\build_all.cmd`, which builds x64 and ARM64 drivers/companions and the
x64 OpenVR payload, then builds the managed SDK twice. The first pass stages
resources; the second embeds them. The upstream resource target requires both
native architectures, WDK signing/catalog tools, and verified pinned usbip-win2
installer payloads. A standalone bridge build or restore cannot replace this
native preparation.

Next the script publishes the bridge in Release for `win-x64` with
`--self-contained true`, builds `hidmaestro-mcp` with `--release --locked`, and
copies the entire publish directory, MCP executable, licenses and unsigned notice.

| Option | Default / use |
|---|---|
| `-Mode Package` | Default: stage, ZIP, and SHA-256 file |
| `-Mode Stage` | Build/stage only; print `Staged bundle:` and exit without archiving |
| `-Version` | Defaults to `workspace.package.version` in root `Cargo.toml`; overrides the bundle name, not package versions |
| `-HMRepoRoot` | Defaults to `HM_REPO_ROOT`, then the pinned `vendor\HIDMaestro`; alternate checkout for intentional upstream integration work |
| `-SkipUpstreamBuild` | Skip only the upstream build; requires already-built, current native/embedded payloads |
| `-OutputDirectory` | Defaults to repository `artifacts`; relative overrides resolve from the calling directory |

At version 0.1.0, default outputs are:

```text
artifacts/stage/hidmaestro-rs-0.1.0-win-x64/
artifacts/hidmaestro-rs-0.1.0-win-x64.zip
artifacts/hidmaestro-rs-0.1.0-win-x64.zip.sha256
```

The selected stage directory is replaced on each run. The bridge publish source
is `bridge/HIDMaestro.Bridge/bin/Release/net10.0-windows10.0.26100.0/win-x64/publish`;
the Rust binary is expected at `target/release/hidmaestro-mcp.exe`. Nondefault
Cargo target/output settings must be accounted for because the script uses that
fixed binary location.

For direct MSBuild work, bridge property `HMRepoRoot` overrides environment
`HM_REPO_ROOT`, then falls back to the submodule. These are build-only settings,
not runtime bridge-discovery variables. Upstream native scripts additionally
accept `HM_WDK_VERSION` and paired `HM_ARM64_COMPILER_DIR`/`HM_ARM64_CRT_DIR`;
the SDK resource pack still explicitly references 10.0.26100.0 tools, so changing
the native override alone does not change all kit dependencies.

## Manual elevated smoke validation

Use a dedicated Windows x64 host available for mutation. The
[smoke script](../scripts/smoke-windows.ps1) can install the driver and calls
`remove_all_controllers` **before and after** its checks. That reaches upstream
system-wide HIDMaestro cleanup, including controllers outside this task and
attempted removal of driver packages/configuration. It is not a read-only test
or a safe default on a shared active host.

Prerequisites: a complete bundle, elevated 64-bit PowerShell 7, and a free
XInput slot among 0–3 after the initial sweep. Building that bundle requires
the WDK/native tools above; running an already-built bundle does not. Unset
`HIDMAESTRO_PIPE_NAME` and `HIDMAESTRO_BRIDGE_PATH` for this procedure so the
script's MCP process spawns the staged sibling bridge with inherited elevation.

From the repository root in that dedicated elevated session:

```powershell
pwsh -File scripts/package-windows.ps1 -Mode Stage
pwsh -File scripts/smoke-windows.ps1
```

`-BundleDirectory` can name the bundle directly or a parent containing exactly
one bundle (default `artifacts\stage`). `-TimeoutSeconds` is 5–120, default 30;
individual MCP response waits are capped at 30 seconds by the script.

The script initializes legacy MCP, broadly cleans devices, records existing
XInput slots, checks/installs the driver, loads profiles, and creates an
`xbox-360-wired` controller. It obtains the key from structured controller data,
waits for a newly occupied XInput slot, holds A, observes A through
`xinput1_4.dll`/`XInputGetState`, releases A, and observes release while the target
slot remains connected. It does not test sticks, feedback, all profiles, or all
Windows input APIs.

In `finally`, it attempts to reset its key, performs broad cleanup again,
requests bridge shutdown, closes MCP stdin, and waits for process exit (with a
kill fallback). Cleanup failures are warnings, not proof all cleanup succeeded.
The manual-only **Windows elevated smoke** workflow performs this procedure on
the self-hosted `hidmaestro-release` runner.

## CI and releases

[Ordinary CI](../.github/workflows/ci.yml) runs Rust checks on hosted Ubuntu.
Hosted Windows builds/tests Rust, restores the bridge's .NET project,
syntax-checks the packaging script, and verifies the pinned submodule commit.
It does not run packaging, compile the native driver, install devices, or run
XInput validation. A passing restore does not establish that `Program.cs` and
the upstream resource-pack build succeeded.

Release Please owns version bumps, `vX.Y.Z` tags, changelog updates, and GitHub
releases on `main`, using [its configuration](../release-please-config.json)
and [manifest](../.release-please-manifest.json). Do not create versions, tags,
or releases by hand. The [release workflow](../.github/workflows/release-please.yml)
runs Release Please on hosted Ubuntu. After a release is created, its packaging
job uses a maintained self-hosted Windows x64 runner labelled
`hidmaestro-release`, with MSVC/WDK prerequisites, to build and attach the
unsigned ZIP and checksum.

The workflow also configures a hosted Ubuntu `publish-crate` job to publish
`hidmaestro` from the release tag with `cargo publish --package hidmaestro --locked`;
it needs the `CARGO_REGISTRY_TOKEN` repository secret. Configured jobs do not
prove that an artifact build or crate publication has occurred. Check actual
release assets/publication before changing the installation guides to recommend
downloads or crates.io. The current [setup guide](getting-started.md#build-your-installation)
therefore leads with a source build.
