//! Bridge process management and RPC client.

use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::protocol::{Inbound, Request};
use crate::state::{
    Axis, Buttons, ControllerInfo, GamepadState, Hat, OutputEvent, Profile, StandardAxes,
};

/// Environment variable naming the bridge executable.
pub const BRIDGE_PATH_ENV: &str = "HIDMAESTRO_BRIDGE_PATH";

/// Default binary name searched on `PATH` when no explicit path is given.
pub const BRIDGE_BIN_NAME: &str = "hidmaestro-bridge";

/// Builder for [`HidMaestro`].
#[derive(Debug, Clone)]
pub struct HidMaestroBuilder {
    bridge_path: Option<PathBuf>,
    args: Vec<String>,
    response_timeout: Duration,
    env: Vec<(String, String)>,
}

impl Default for HidMaestroBuilder {
    fn default() -> Self {
        Self {
            bridge_path: None,
            args: Vec::new(),
            response_timeout: Duration::from_secs(120),
            env: Vec::new(),
        }
    }
}

impl HidMaestroBuilder {
    /// Explicit path to the bridge executable. Overrides
    /// `HIDMAESTRO_BRIDGE_PATH` and `PATH` lookup.
    pub fn bridge_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.bridge_path = Some(path.into());
        self
    }

    /// Extra command-line arguments passed to the bridge process.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Extra environment variable set on the bridge process.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// How long a single RPC may wait for the bridge's response.
    /// Default 120 s (driver install can be slow on first run).
    pub fn response_timeout(mut self, timeout: Duration) -> Self {
        self.response_timeout = timeout;
        self
    }

    fn resolve_path(&self) -> Result<PathBuf> {
        if let Some(p) = &self.bridge_path {
            return Ok(p.clone());
        }
        if let Ok(p) = env::var(BRIDGE_PATH_ENV) {
            if !p.is_empty() {
                return Ok(PathBuf::from(p));
            }
        }
        let exe = if cfg!(windows) {
            format!("{BRIDGE_BIN_NAME}.exe")
        } else {
            BRIDGE_BIN_NAME.to_string()
        };
        let path_var = env::var_os("PATH").unwrap_or_default();
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(&exe);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(Error::BridgeNotFound(format!(
            "set {BRIDGE_PATH_ENV}, pass builder().bridge_path(...), or put `{exe}` on PATH"
        )))
    }

    /// Spawn the bridge process and return a connected client.
    ///
    /// Note: the bridge (and the virtual-driver machinery behind it) only
    /// functions on Windows; on other platforms calls fail unless the
    /// pointed-to bridge is a compatible stand-in.
    pub fn spawn(self) -> Result<HidMaestro> {
        let program = self.resolve_path()?;
        let mut cmd = Command::new(&program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|source| Error::Spawn {
            program: program.display().to_string(),
            source,
        })?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let (tx, rx) = channel::<Inbound>();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Inbound>(&line) {
                    Ok(msg) => {
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                    // Non-JSON noise on stdout (SDK warnings, etc.): ignore.
                    Err(_) => continue,
                }
            }
            // Sender dropped here → calls fail with BridgeExited.
        });

        Ok(HidMaestro {
            child: Some(child),
            stdin,
            rx,
            next_id: AtomicU64::new(1),
            pending_events: VecDeque::new(),
            controllers: HashMap::new(),
            response_timeout: self.response_timeout,
        })
    }
}

struct ControllerSlot {
    info: ControllerInfo,
    state: GamepadState,
}

/// A connected HIDMaestro session. Wraps the bridge process; dropping it
/// kills the child (which tears down all virtual controllers).
///
/// Create via [`HidMaestro::builder`]`().spawn()` or [`HidMaestro::spawn`].
pub struct HidMaestro {
    child: Option<Child>,
    stdin: ChildStdin,
    rx: Receiver<Inbound>,
    next_id: AtomicU64,
    pending_events: VecDeque<OutputEvent>,
    controllers: HashMap<String, ControllerSlot>,
    response_timeout: Duration,
}

impl HidMaestro {
    pub fn builder() -> HidMaestroBuilder {
        HidMaestroBuilder::default()
    }

    /// Spawn with default settings: bridge found via `HIDMAESTRO_BRIDGE_PATH`
    /// or `PATH`.
    pub fn spawn() -> Result<Self> {
        Self::builder().spawn()
    }

    fn rpc<P: Serialize>(
        &mut self,
        method: &'static str,
        params: Option<P>,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request { id, method, params };
        let mut line = serde_json::to_vec(&req)?;
        line.push(b'\n');
        self.stdin.write_all(&line).map_err(|e| {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                Error::BridgeExited(String::new())
            } else {
                Error::Io(e)
            }
        })?;
        self.stdin.flush()?;

        let deadline = Instant::now() + self.response_timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            match self.rx.recv_timeout(deadline - now) {
                Ok(Inbound::Response {
                    id: rid,
                    ok,
                    result,
                    error,
                }) if rid == id => {
                    return if ok {
                        Ok(result.unwrap_or(serde_json::Value::Null))
                    } else {
                        Err(Error::Remote(
                            error.unwrap_or_else(|| "unknown error".into()),
                        ))
                    };
                }
                Ok(Inbound::Response { .. }) => continue,
                Ok(Inbound::Event { data, .. }) => {
                    if let Ok(ev) = serde_json::from_value::<OutputEvent>(data) {
                        self.pending_events.push_back(ev);
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(Error::Timeout),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let status = self
                        .child
                        .as_mut()
                        .and_then(|c| c.try_wait().ok().flatten())
                        .map(|s| format!(" (status {s})"))
                        .unwrap_or_default();
                    return Err(Error::BridgeExited(status));
                }
            }
        }
    }

    /// Drain decoded output reports (rumble/FFB/LED writes from games) that
    /// arrived while other calls were running.
    pub fn drain_events(&mut self) -> Vec<OutputEvent> {
        self.pending_events.drain(..).collect()
    }

    /// Block until the next output event or `timeout`.
    pub fn wait_event(&mut self, timeout: Duration) -> Result<Option<OutputEvent>> {
        if let Some(ev) = self.pending_events.pop_front() {
            return Ok(Some(ev));
        }
        match self.rx.recv_timeout(timeout) {
            Ok(Inbound::Event { data, .. }) => Ok(serde_json::from_value(data).ok()),
            Ok(Inbound::Response { .. }) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    /// Round-trip check that the bridge is alive.
    pub fn ping(&mut self) -> Result<String> {
        let v = self.rpc("ping", None::<()>)?;
        Ok(v.as_str().unwrap_or("pong").to_string())
    }

    /// True when the HIDMaestro driver package is already installed.
    pub fn is_driver_installed(&mut self) -> Result<bool> {
        Ok(self
            .rpc("is_driver_installed", None::<()>)?
            .as_bool()
            .unwrap_or(false))
    }

    /// Extract, sign, and install the UMDF2 driver package. Requires
    /// elevation (administrator) on first run.
    pub fn install_driver(&mut self) -> Result<()> {
        self.rpc("install_driver", None::<()>)?;
        Ok(())
    }

    /// Install the optional usbip-win2 backend (needed by `-composite`
    /// profiles with audio endpoints).
    pub fn install_usbip_backend(&mut self) -> Result<()> {
        self.rpc("install_usbip_backend", None::<()>)?;
        Ok(())
    }

    /// Load the built-in profile catalog (~230 controllers).
    pub fn load_default_profiles(&mut self) -> Result<usize> {
        let v = self.rpc("load_profiles", None::<()>)?;
        Ok(v.as_u64().unwrap_or(0) as usize)
    }

    /// Load profile JSONs from a directory instead of the built-in catalog.
    pub fn load_profiles_from_dir(&mut self, dir: impl Into<String>) -> Result<usize> {
        #[derive(Serialize)]
        struct P {
            dir: String,
        }
        let v = self.rpc("load_profiles", Some(P { dir: dir.into() }))?;
        Ok(v.as_u64().unwrap_or(0) as usize)
    }

    /// All loaded profiles.
    pub fn list_profiles(&mut self) -> Result<Vec<Profile>> {
        let v = self.rpc("list_profiles", None::<()>)?;
        Ok(serde_json::from_value(v)?)
    }

    /// One profile by id, if loaded.
    pub fn get_profile(&mut self, id: &str) -> Result<Option<Profile>> {
        #[derive(Serialize)]
        struct P<'a> {
            id: &'a str,
        }
        let v = self.rpc("get_profile", Some(P { id }))?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(v)?))
    }

    /// Create a virtual controller from a profile id (e.g.
    /// `"xbox-360-wired"`, `"dualsense"`, `"switch-pro"`). Returns its key.
    ///
    /// `index` pins the controller to an enumeration slot; `None` appends.
    /// Use [`HidMaestro::controller`] to drive it.
    pub fn create_controller(&mut self, profile_id: &str, index: Option<usize>) -> Result<String> {
        #[derive(Serialize)]
        struct P<'a> {
            profile_id: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            index: Option<usize>,
        }
        let v = self.rpc("create_controller", Some(P { profile_id, index }))?;
        let key = v
            .get("key")
            .and_then(|k| k.as_str())
            .ok_or_else(|| Error::Remote(format!("bad create_controller result: {v}")))?
            .to_string();
        self.controllers.insert(
            key.clone(),
            ControllerSlot {
                info: ControllerInfo {
                    key: key.clone(),
                    profile_id: profile_id.to_string(),
                },
                state: GamepadState::neutral(),
            },
        );
        Ok(key)
    }

    /// A mutable view of one live controller. The borrow is transient:
    /// call `hm.controller("c1")` again whenever you need it.
    pub fn controller(&mut self, key: &str) -> Result<Controller<'_>> {
        if !self.controllers.contains_key(key) {
            return Err(Error::NoSuchController(key.to_string()));
        }
        Ok(Controller {
            hm: self,
            key: key.to_string(),
        })
    }

    /// Controllers currently alive in the bridge session.
    pub fn list_controllers(&mut self) -> Result<Vec<ControllerInfo>> {
        let v = self.rpc("list_controllers", None::<()>)?;
        Ok(serde_json::from_value(v)?)
    }

    /// Submit a state to a controller by key (stateless counterpart to
    /// [`Controller::submit`]).
    pub fn submit_state(&mut self, key: &str, state: &GamepadState) -> Result<()> {
        #[derive(Serialize)]
        struct P<'a> {
            key: &'a str,
            state: &'a GamepadState,
        }
        self.rpc("submit_state", Some(P { key, state }))?;
        if let Some(slot) = self.controllers.get_mut(key) {
            slot.state = state.clone();
        }
        Ok(())
    }

    /// The client's mirrored state for a controller key.
    pub fn state(&self, key: &str) -> Option<&GamepadState> {
        self.controllers.get(key).map(|s| &s.state)
    }

    /// Mutable access to the mirrored state. Call
    /// [`HidMaestro::submit_state`] (or [`Controller::submit`]) to push it.
    pub fn state_mut(&mut self, key: &str) -> Option<&mut GamepadState> {
        self.controllers.get_mut(key).map(|s| &mut s.state)
    }

    /// Remove a controller by key (hot-unplug).
    pub fn remove_controller(&mut self, key: &str) -> Result<()> {
        #[derive(Serialize)]
        struct P<'a> {
            key: &'a str,
        }
        self.rpc("remove_controller", Some(P { key }))?;
        self.controllers.remove(key);
        Ok(())
    }

    /// Remove every virtual controller created by this session's driver
    /// install (including strays from previous unclean exits).
    pub fn remove_all_controllers(&mut self) -> Result<()> {
        self.rpc("remove_all_controllers", None::<()>)?;
        self.controllers.clear();
        Ok(())
    }

    /// Stop the bridge process (disposes all controllers).
    pub fn shutdown(mut self) {
        let _ = self.rpc("shutdown", None::<()>);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for HidMaestro {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Mutable view of one live virtual controller. Mutating methods update the
/// client's mirrored [`GamepadState`]; `submit`-style methods push it to the
/// device. Most helpers submit automatically.
///
/// The handle borrows the session — obtain it transiently via
/// [`HidMaestro::controller`], don't store it.
pub struct Controller<'a> {
    hm: &'a mut HidMaestro,
    key: String,
}

impl<'a> Controller<'a> {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn info(&self) -> &ControllerInfo {
        &self.hm.controllers[&self.key].info
    }

    pub fn state(&self) -> &GamepadState {
        &self.hm.controllers[&self.key].state
    }

    fn state_mut(&mut self) -> &mut GamepadState {
        &mut self.hm.controllers.get_mut(&self.key).unwrap().state
    }

    /// Push the mirrored state to the virtual device.
    pub fn submit(&mut self) -> Result<()> {
        let state = self.state().clone();
        self.hm.submit_state(&self.key, &state)
    }

    /// Replace the whole state and submit.
    pub fn submit_state(&mut self, state: GamepadState) -> Result<()> {
        self.state_mut().clone_from(&state);
        self.submit()
    }

    /// Reset to neutral and submit.
    pub fn reset(&mut self) -> Result<()> {
        self.submit_state(GamepadState::neutral())
    }

    /// Hold buttons down (without releasing others) and submit.
    pub fn press(&mut self, buttons: Buttons) -> Result<()> {
        self.state_mut().buttons |= buttons;
        self.submit()
    }

    /// Release buttons (leaving others held) and submit.
    pub fn release(&mut self, buttons: Buttons) -> Result<()> {
        self.state_mut().buttons -= buttons;
        self.submit()
    }

    /// Press, hold for `hold`, release. Submits twice.
    pub fn tap(&mut self, buttons: Buttons, hold: Duration) -> Result<()> {
        self.press(buttons)?;
        thread::sleep(hold);
        self.release(buttons)
    }

    /// Set the d-pad direction and submit.
    pub fn set_hat(&mut self, hat: Hat) -> Result<()> {
        let st = self.state_mut();
        st.hat = hat;
        st.hat_degrees = None;
        self.submit()
    }

    /// Set one analog axis (0.0..=1.0) by HID usage and submit.
    pub fn set_axis(&mut self, axis: Axis, value: f32) -> Result<()> {
        self.state_mut().axes.insert(axis, value.clamp(0.0, 1.0));
        self.submit()
    }

    /// Set the canonical six axes (sticks centered at 0.5, triggers 0..1)
    /// and submit.
    pub fn set_standard_axes(&mut self, axes: StandardAxes) -> Result<()> {
        self.state_mut().standard_axes = Some(axes);
        self.submit()
    }

    /// Remove this controller (hot-unplug). Consumes the handle.
    pub fn remove(self) -> Result<()> {
        self.hm.remove_controller(&self.key)
    }
}
