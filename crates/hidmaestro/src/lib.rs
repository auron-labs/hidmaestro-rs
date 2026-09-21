//! # hidmaestro
//!
//! Rust client for [HIDMaestro](https://github.com/hifihedgehog/HIDMaestro),
//! virtual game controllers that look like real hardware to every Windows
//! input API (DirectInput, XInput, SDL3, browser Gamepad, WGI).
//!
//! The crate either manages a small .NET bridge process (`hidmaestro-bridge`,
//! see `bridge/HIDMaestro.Bridge` in this repository) or connects to its
//! Windows named pipe, and talks newline-delimited JSON-RPC to it. Use it to
//! spin up virtual controllers inside end-to-end tests of software that
//! requires a gamepad.
//!
//! ```no_run
//! use hidmaestro::{Buttons, Hat, HidMaestro, StandardAxes};
//! use std::time::Duration;
//!
//! # fn main() -> hidmaestro::Result<()> {
//! let mut hm = HidMaestro::spawn()?; // bridge via HIDMAESTRO_BRIDGE_PATH, sibling executable, or PATH
//! if !hm.is_driver_installed()? {
//!     hm.install_driver()?;                  // needs elevation on first run
//! }
//! hm.load_default_profiles()?;
//!
//! let key = hm.create_controller("xbox-360-wired", None)?;
//! let mut pad = hm.controller(&key)?;
//! pad.tap(Buttons::A, Duration::from_millis(80))?;
//! pad.set_standard_axes(StandardAxes {
//!     left_stick_x: Some(1.0),               // full right
//!     ..Default::default()
//! })?;
//! pad.set_hat(Hat::North)?;
//! pad.remove()?;                             // hot-unplug
//! # Ok(())
//! # }
//! ```
//!
//! The bridge and driver only run on Windows; this crate itself compiles
//! everywhere so tests can `cfg`-gate driver-dependent paths.

mod client;
mod error;
mod protocol;
mod state;

pub use client::{
    Controller, HidMaestro, HidMaestroBuilder, BRIDGE_BIN_NAME, BRIDGE_PATH_ENV, PIPE_NAME_ENV,
};
pub use error::{Error, Result};
pub use state::{
    Axis, Buttons, ControllerInfo, GamepadState, Hat, OutputEvent, Profile, StandardAxes,
};

#[cfg(test)]
mod tests;
