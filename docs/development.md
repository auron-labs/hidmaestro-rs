# Development and releases

## Prerequisites

Rust and .NET 10 are required for ordinary source checks. Building the Windows
bundle additionally requires Windows x64, Visual Studio 2022 Build Tools with
the MSVC x64 tools, Windows SDK/WDK 10.0.26100.0, and PowerShell. The pinned
HIDMaestro source is the `vendor/HIDMaestro` submodule.

```powershell
git clone --recurse-submodules https://github.com/auron-labs/hidmaestro-rs.git
cd hidmaestro-rs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

`HMRepoRoot` (MSBuild property) and `HM_REPO_ROOT` (environment variable) can
point the bridge at another HIDMaestro checkout. The in-tree submodule is the
default.

## Windows bundle

On a suitably provisioned Windows x64 machine, create the release archive:

```powershell
pwsh -File scripts/package-windows.ps1
```

The script first runs `vendor\HIDMaestro\scripts\build_all.cmd`, publishes the
self-contained bridge, builds the release MCP executable, and writes
`artifacts\hidmaestro-rs-<version>-win-x64.zip` plus its `.sha256` file.
Use `-Mode Stage` to leave the same deterministic bundle directory under
`artifacts\stage` without creating an archive. `-SkipUpstreamBuild` is only for
a checkout whose upstream driver and SDK payload were already built.

The hosted Windows CI runner restores the bridge and builds/tests the Rust
workspace, but does not run this package script or compile the upstream driver.
That native build requires the Windows Driver Kit 10.0.26100.0 and Visual Studio
native toolchain; driver installation also requires an elevated Windows session.
Those prerequisites are not provided or safely usable on GitHub-hosted runners.
It also cannot compile `Program.cs` independently: its project reference builds
HIDMaestro.Core, whose resource-pack target explicitly fails until the native
driver payload has been built.
Release Please itself runs on GitHub-hosted Ubuntu runners, so release PR
maintenance does not need that machine. Only the dependent artifact-packaging
job runs on a maintained self-hosted Windows x64 runner labelled
`hidmaestro-release` with the WDK and MSVC tools.

## Manual elevated smoke validation

Ordinary CI does not install a driver or create a virtual controller. The
manual-only **Windows elevated smoke** workflow uses the `hidmaestro-release`
runner to package a bundle, drive its MCP executable, and verify an Xbox 360
controller through XInput. Run it only when the runner is available for host
mutation.

Locally, from an elevated 64-bit PowerShell on a WDK-equipped Windows x64
machine, stage the bundle and run the same smoke check:

```powershell
pwsh -File scripts/package-windows.ps1 -Mode Stage
pwsh -File scripts/smoke-windows.ps1
```

The smoke script resets and removes its virtual controllers and shuts down the
MCP bridge in `finally`; stdin EOF is also a supported server shutdown path.

## Releases

Release Please is the sole owner of version bumps, `vX.Y.Z` tags, changelog
updates, and GitHub releases on `main`. After its release PR is merged, the
release workflow builds the unsigned Windows x64 archive and checksum and
attaches both to the created GitHub release. Do not create tags or releases by
hand.
