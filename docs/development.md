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
cargo publish --package hidmaestro --dry-run --locked
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
| `-Version` | Defaults to `workspace.package.version` in root `Cargo.toml`; an explicit value must match. Stamps the bridge's version and informational version as well as bundle names. |
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

Use the **Windows elevated smoke** workflow on `main` to build and test on a
fresh GitHub-hosted `windows-2025` VM. You do not need to connect your own
Windows computer or configure a GitHub environment. The workflow prepares the
driver build tools, packages the ZIP, tests its extracted contents, and retains
the bundle and logs. The VM is discarded after the job.

For optional local testing, use only a disposable Windows x64 test host. The
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

For local testing, from the repository root in that elevated session, package the ZIP
and validate a fresh extraction:

```powershell
pwsh -File scripts/package-windows.ps1
pwsh -File scripts/test-bundle-windows.ps1 -ArchivePath artifacts/hidmaestro-rs-0.1.0-win-x64.zip -Version 0.1.0 -ExtractionDirectory artifacts/smoke-extracted
```

Substitute the workspace version for `0.1.0`. The extraction directory must not
already exist. The validation wrapper checks the ZIP checksum, required files,
and bridge informational version before running the native smoke script against
the extracted files. The smoke script also checks the MCP-reported version. The
wrapper clears bridge/pipe overrides and requires the accompanying checksum.

`-BundleDirectory` can name the bundle directly or a parent containing exactly
one bundle (default `artifacts\stage`). `-TimeoutSeconds` is 5–120, default 30;
individual MCP response waits are capped at 30 seconds by the script.
`-ExpectedVersion` optionally checks the MCP version returned by initialization;
the extracted-ZIP wrapper always supplies it.

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
This manual workflow is separate from release publication. The release
workflow builds and stages the bundle but does not install drivers or run
XInput validation. Neither workflow needs an owner-supplied Windows runner.

[Runner setup](../scripts/setup-windows-runner.ps1) checks the preinstalled
Visual Studio x64/ARM64 tools and SDK 10.0.26100.0, and installs Microsoft's
WDK 26100.6584 when required files are absent. It verifies the installer's
Microsoft Authenticode signature and required build files. The WDK revision
uses the `10.0.26100.0` kit directory expected by the pinned upstream scripts.
See GitHub's [Windows 2025 image inventory](https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md)
and Microsoft's [WDK downloads](https://learn.microsoft.com/windows-hardware/drivers/other-wdk-downloads).
This provisioning script refuses to run outside a GitHub-hosted Windows job.

## CI and releases

[Ordinary CI](../.github/workflows/ci.yml) runs Rust checks on hosted Ubuntu.
Hosted Windows builds/tests Rust, restores the bridge's .NET project,
syntax-checks the packaging script, and verifies the pinned submodule commit.
It does not run packaging, compile the native driver, install devices, or run
XInput validation. A passing restore does not establish that `Program.cs` and
the upstream resource-pack build succeeded.

Release Please owns version bumps, `vX.Y.Z` tags, changelogs, and draft GitHub
releases on `main`, using [its configuration](../release-please-config.json)
and [manifest](../.release-please-manifest.json). Do not create versions, tags,
or releases by hand. [GoReleaser](../.goreleaser.yaml) packages the staged Windows
bundle, generates its checksum, and uploads both to the draft. The existing
PowerShell packager continues to own the native/.NET/Rust build.

`workspace.package.version` is the package/bundle source of truth. Release Please
uses its `simple` strategy because its Rust updater cannot update this virtual
workspace with inherited member versions. The generated
[version marker](../.release-please-version) is release metadata. TOML selectors
update the workspace version and the `hidmaestro` and `hidmaestro-mcp` entries in
`Cargo.lock`. The lockfile selector uses `name.value` because
[Release Please's TOML parser](https://github.com/googleapis/release-please/blob/v17.6.0/src/util/toml-edit.ts)
wraps scalar values in tagged objects; registry/git dependencies are excluded.
No lockfile comment markers need to be preserved when Cargo rewrites it.

The [release workflow](../.github/workflows/release-please.yml) has three jobs:

1. **Release Please** opens/updates the release PR. Merging it creates the tag
   and draft release.
2. **Package** checks out that tag on GitHub-hosted Windows, checks versions,
   provisions the native build tools, and runs `package-windows.ps1 -Mode Stage`.
   It retains the complete staging directory and build logs as workflow artifacts.
3. **Publish** checks out the same tag on Ubuntu, dry-runs crate publication,
   and uses GoReleaser to ZIP the staged files and upload the ZIP/checksum to the
   draft. It publishes `hidmaestro` with Cargo, then publishes the GitHub release.

The Windows job has read-only repository permissions and no publishing token.
The Ubuntu publication job uses `GITHUB_TOKEN` and the configured
`CARGO_REGISTRY_TOKEN` with permission to publish `hidmaestro`. Release Please
and publication stay in one workflow, so a tag push does not need to trigger a
second workflow. GoReleaser OSS is sufficient; no Pro key is needed.

GoReleaser preserves the existing bundle directory layout and asset names:
`hidmaestro-rs-X.Y.Z-win-x64.zip` and its `.zip.sha256`. It keeps Release Please's
release notes and refuses to replace existing assets. It does not rebuild the
native payload or validate controller behavior. The optional
[manual Windows smoke workflow](#manual-elevated-smoke-validation) is separate.
A successful hosted native build and XInput run are still unverified until an
actual run establishes them; configuration alone is not evidence of success.

### Check release configuration locally

With the configured tools available (`mise install`):

```sh
goreleaser check
bash scripts/test-goreleaser.sh
```

The packaging check uses a disposable local Git repository and staged fixture
files. It exercises the real GoReleaser configuration, verifies nested runtime
files survive extraction, and checks the ZIP's SHA-256. It builds no native code
and skips all publication. Hosted Ubuntu CI runs this check.

On PowerShell 7, `pwsh -File scripts/test-release.ps1` checks workspace/release
version consistency, tag mismatch rejection, smoke-response parsing, and checksum
rejection without running the native smoke body. Hosted Windows CI runs it.
`pwsh -File scripts/check-release-version.ps1 -Tag vX.Y.Z` also validates an
expected release tag directly. The version check resolves locked Cargo metadata
and compares workspace versions with Release Please's manifest and version marker.

### Handle a failed release

A failed build stops publication. GoReleaser leaves the release as a draft until
crate publication succeeds, and build logs are retained for diagnosis. Rerun
failed jobs for build failures. If assets or the crate were already uploaded,
inspect the partial release and finish the remaining publication steps manually;
there is no custom automatic recovery system. Do not move the tag, replace
published assets, or publish a new version merely to retry a failed step.

GitHub and crates.io cannot be published atomically: the crate may be public if
the final GitHub publication step fails. In that case, finish publishing the
existing draft rather than publishing the crate again. Recommend downloads and
crates.io only after checking actual public assets and the published crate.
Until then, the [setup guide](getting-started.md#build-your-installation) stays
source-first.
