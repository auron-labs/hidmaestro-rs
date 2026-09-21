//! Bridge process management and RPC client.

use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};
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

/// Environment variable naming an already-running elevated bridge's Windows
/// named pipe.
pub const PIPE_NAME_ENV: &str = "HIDMAESTRO_PIPE_NAME";

/// Default binary name searched on `PATH` when no explicit path is given.
pub const BRIDGE_BIN_NAME: &str = "hidmaestro-bridge";

// HMContext disposal can take 5-11 seconds while Windows removes devices.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_EVENT_BUFFER_CAPACITY: usize = 1_024;
const MAX_PIPE_NAME_LENGTH: usize = 128;

/// Builder for [`HidMaestro`].
#[derive(Debug, Clone)]
pub struct HidMaestroBuilder {
    bridge_path: Option<PathBuf>,
    pipe_name: Option<String>,
    args: Vec<String>,
    response_timeout: Duration,
    event_buffer_capacity: usize,
    env: Vec<(String, String)>,
}

impl Default for HidMaestroBuilder {
    fn default() -> Self {
        Self {
            bridge_path: None,
            pipe_name: None,
            args: Vec::new(),
            response_timeout: Duration::from_secs(120),
            event_buffer_capacity: DEFAULT_EVENT_BUFFER_CAPACITY,
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

    /// Connect to an already-running elevated bridge on this Windows named
    /// pipe instead of spawning a bridge process. Overrides
    /// [`PIPE_NAME_ENV`].
    pub fn pipe_name(mut self, name: impl Into<String>) -> Self {
        self.pipe_name = Some(name.into());
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

    /// Maximum decoded output events retained while the application is not
    /// draining them. When full, the oldest event is discarded so bridge RPC
    /// responses can always continue flowing.
    ///
    /// `capacity` must be greater than zero.
    pub fn event_buffer_capacity(mut self, capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "event buffer capacity must be greater than zero"
        );
        self.event_buffer_capacity = capacity;
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
        if let Some(path) = bundled_bridge_path(&exe) {
            return Ok(path);
        }
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

    fn configured_pipe_name(&self, environment_pipe_name: Option<&str>) -> Result<Option<String>> {
        match self.pipe_name.as_deref() {
            Some(name) => validate_pipe_name(name).map(Some),
            None => environment_pipe_name
                .filter(|name| !name.is_empty())
                .map(validate_pipe_name)
                .transpose(),
        }
    }

    fn pipe_name_from_environment(&self) -> Result<Option<String>> {
        self.configured_pipe_name(env::var(PIPE_NAME_ENV).ok().as_deref())
    }

    /// Spawn the bridge process and return a connected client.
    ///
    /// Note: the bridge (and the virtual-driver machinery behind it) only
    /// functions on Windows; on other platforms calls fail unless the
    /// pointed-to bridge is a compatible stand-in.
    pub fn spawn(self) -> Result<HidMaestro> {
        if let Some(pipe_name) = self.pipe_name_from_environment()? {
            return self.connect_pipe(&pipe_name);
        }
        self.spawn_bridge()
    }

    fn spawn_bridge(self) -> Result<HidMaestro> {
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

        Ok(HidMaestro::from_streams(
            child.stdin.take().expect("piped stdin"),
            child.stdout.take().expect("piped stdout"),
            Some(child),
            self.event_buffer_capacity,
            self.response_timeout,
        ))
    }

    #[cfg(windows)]
    fn connect_pipe(self, pipe_name: &str) -> Result<HidMaestro> {
        use std::fs::OpenOptions;

        let pipe_path = format!(r"\\.\pipe\{pipe_name}");
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
            .map_err(|source| Error::PipeConnect {
                name: pipe_name.to_string(),
                source,
            })?;
        let reader = writer.try_clone().map_err(|source| Error::PipeConnect {
            name: pipe_name.to_string(),
            source,
        })?;
        Ok(HidMaestro::from_streams(
            writer,
            reader,
            None,
            self.event_buffer_capacity,
            self.response_timeout,
        ))
    }

    #[cfg(not(windows))]
    fn connect_pipe(self, _pipe_name: &str) -> Result<HidMaestro> {
        Err(Error::PipeUnsupported)
    }
}

fn bundled_bridge_path(executable_name: &str) -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .as_deref()
        .and_then(|current_exe| bridge_next_to(current_exe, executable_name))
}

fn bridge_next_to(current_exe: &Path, executable_name: &str) -> Option<PathBuf> {
    let candidate = current_exe.parent()?.join(executable_name);
    candidate.is_file().then_some(candidate)
}

fn validate_pipe_name(name: &str) -> Result<String> {
    let reason = if name.is_empty() {
        Some("name must not be empty")
    } else if name.len() > MAX_PIPE_NAME_LENGTH {
        Some("name must be at most 128 ASCII characters")
    } else if !name
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
    {
        Some("name must start with an ASCII letter or digit")
    } else if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Some(
            r"name may contain only ASCII letters, digits, '-', '_', and '.'; do not include a path or \\.\pipe\ prefix",
        )
    } else {
        None
    };
    match reason {
        Some(reason) => Err(Error::InvalidPipeName {
            name: name.to_string(),
            reason,
        }),
        None => Ok(name.to_string()),
    }
}

struct ControllerSlot {
    info: ControllerInfo,
    state: GamepadState,
}

struct EventQueue {
    events: VecDeque<OutputEvent>,
    dropped: u64,
}

struct EventBuffer {
    queue: Mutex<EventQueue>,
    available: Condvar,
    capacity: usize,
}

impl EventBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(EventQueue {
                events: VecDeque::new(),
                dropped: 0,
            }),
            available: Condvar::new(),
            capacity,
        }
    }

    fn push(&self, event: OutputEvent) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if queue.events.len() == self.capacity {
            queue.events.pop_front();
            queue.dropped += 1;
        }
        queue.events.push_back(event);
        self.available.notify_one();
    }

    fn drain(&self) -> Vec<OutputEvent> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.events.drain(..).collect()
    }

    fn wait(&self, timeout: Duration) -> Option<OutputEvent> {
        self.wait_with(timeout, |_| true)
    }

    fn wait_for_controller(&self, controller: &str, timeout: Duration) -> Option<OutputEvent> {
        self.wait_with(timeout, |event| event.controller == controller)
    }

    fn wait_with(
        &self,
        timeout: Duration,
        matches: impl Fn(&OutputEvent) -> bool,
    ) -> Option<OutputEvent> {
        let deadline = Instant::now() + timeout;
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(index) = queue.events.iter().position(&matches) {
                return queue.events.remove(index);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (new_queue, result) = self
                .available
                .wait_timeout(queue, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue = new_queue;
            if result.timed_out() {
                return queue
                    .events
                    .iter()
                    .position(&matches)
                    .and_then(|index| queue.events.remove(index));
            }
        }
    }

    fn dropped(&self) -> u64 {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .dropped
    }
}

/// A connected HIDMaestro session. Wraps the bridge process; dropping it
/// asks the child to dispose all virtual controllers before falling back to
/// termination if it does not exit.
///
/// Create via [`HidMaestro::builder`]`().spawn()` or [`HidMaestro::spawn`].
pub struct HidMaestro {
    child: Option<Child>,
    stdin: Box<dyn Write + Send>,
    rx: Receiver<Inbound>,
    next_id: AtomicU64,
    events: Arc<EventBuffer>,
    controllers: HashMap<String, ControllerSlot>,
    response_timeout: Duration,
    shutdown_sent: bool,
}

impl HidMaestro {
    pub fn builder() -> HidMaestroBuilder {
        HidMaestroBuilder::default()
    }

    /// Spawn with default settings: bridge found via `HIDMAESTRO_BRIDGE_PATH`,
    /// next to the current executable, or on `PATH`.
    pub fn spawn() -> Result<Self> {
        Self::builder().spawn()
    }

    fn from_streams<W, R>(
        stdin: W,
        stdout: R,
        child: Option<Child>,
        event_buffer_capacity: usize,
        response_timeout: Duration,
    ) -> Self
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        let (tx, rx) = channel::<Inbound>();
        let events = Arc::new(EventBuffer::new(event_buffer_capacity));
        let reader_events = Arc::clone(&events);
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Inbound>(&line) {
                    Ok(Inbound::Event { data, .. }) => {
                        if let Ok(event) = serde_json::from_value(data) {
                            reader_events.push(event);
                        }
                    }
                    Ok(message @ Inbound::Response { .. }) => {
                        if tx.send(message).is_err() {
                            break;
                        }
                    }
                    // Non-JSON noise on the protocol stream: ignore.
                    Err(_) => continue,
                }
            }
            // Sender dropped here → calls fail with BridgeExited.
        });

        Self {
            child,
            stdin: Box::new(stdin),
            rx,
            next_id: AtomicU64::new(1),
            events,
            controllers: HashMap::new(),
            response_timeout,
            shutdown_sent: false,
        }
    }

    fn rpc<P: Serialize>(
        &mut self,
        method: &'static str,
        params: Option<P>,
    ) -> Result<serde_json::Value> {
        let id = self.send_request(method, params)?;
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
                // The reader routes events directly into the bounded event
                // buffer. Keep this arm defensive if a future reader changes.
                Ok(Inbound::Event { .. }) => continue,
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

    fn send_request<P: Serialize>(
        &mut self,
        method: &'static str,
        params: Option<P>,
    ) -> Result<u64> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request { id, method, params };
        let mut line = serde_json::to_vec(&req)?;
        line.push(b'\n');
        self.stdin.write_all(&line).map_err(bridge_write_error)?;
        self.stdin.flush().map_err(bridge_write_error)?;
        Ok(id)
    }

    /// Drain all received raw and decoded output reports (rumble/FFB/LED
    /// writes from games), including reports received between RPC calls.
    pub fn drain_events(&mut self) -> Vec<OutputEvent> {
        self.events.drain()
    }

    /// Block until the next output event or `timeout`.
    pub fn wait_event(&mut self, timeout: Duration) -> Result<Option<OutputEvent>> {
        Ok(self.events.wait(timeout))
    }

    /// Block until the next output event for `controller` or `timeout`.
    /// Events for other controllers remain queued for [`Self::drain_events`]
    /// and remain subject to the configured bounded-buffer drop policy.
    pub fn wait_event_for_controller(
        &mut self,
        controller: &str,
        timeout: Duration,
    ) -> Result<Option<OutputEvent>> {
        Ok(self.events.wait_for_controller(controller, timeout))
    }

    /// Number of output events discarded because the configured event buffer
    /// was full. The count is cumulative for the session.
    pub fn dropped_event_count(&self) -> u64 {
        self.events.dropped()
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

    /// Ask the bridge to dispose all controllers. A spawned bridge is then
    /// waited for and terminated only if it does not exit promptly.
    pub fn shutdown(mut self) {
        self.stop_bridge();
    }

    fn stop_bridge(&mut self) {
        if !self.shutdown_sent {
            let _ = self.send_request("shutdown", None::<()>);
            self.shutdown_sent = true;
        }
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + GRACEFUL_SHUTDOWN_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => thread::sleep(SHUTDOWN_POLL_INTERVAL),
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
            }
        }
    }
}

fn bridge_write_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        Error::BridgeExited(String::new())
    } else {
        Error::Io(error)
    }
}

impl Drop for HidMaestro {
    fn drop(&mut self) {
        self.stop_bridge();
    }
}

/// Mutable view of one live virtual controller. Helper methods submit a new
/// [`GamepadState`] and update the client's mirror only after the bridge
/// confirms it, so a failed submission leaves the last confirmed state intact.
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

    /// Push the mirrored state to the virtual device.
    pub fn submit(&mut self) -> Result<()> {
        let state = self.state().clone();
        self.hm.submit_state(&self.key, &state)
    }

    /// Replace the whole state and submit.
    pub fn submit_state(&mut self, state: GamepadState) -> Result<()> {
        self.hm.submit_state(&self.key, &state)
    }

    /// Reset to neutral and submit.
    pub fn reset(&mut self) -> Result<()> {
        self.submit_state(GamepadState::neutral())
    }

    /// Hold buttons down (without releasing others) and submit.
    pub fn press(&mut self, buttons: Buttons) -> Result<()> {
        let mut state = self.state().clone();
        state.buttons |= buttons;
        self.submit_state(state)
    }

    /// Release buttons (leaving others held) and submit.
    pub fn release(&mut self, buttons: Buttons) -> Result<()> {
        let mut state = self.state().clone();
        state.buttons -= buttons;
        self.submit_state(state)
    }

    /// Press, hold for `hold`, release. Submits twice.
    pub fn tap(&mut self, buttons: Buttons, hold: Duration) -> Result<()> {
        self.press(buttons)?;
        thread::sleep(hold);
        self.release(buttons)
    }

    /// Set the d-pad direction and submit.
    pub fn set_hat(&mut self, hat: Hat) -> Result<()> {
        let mut state = self.state().clone();
        state.hat = hat;
        state.hat_degrees = None;
        self.submit_state(state)
    }

    /// Set one analog axis (0.0..=1.0) by HID usage and submit.
    pub fn set_axis(&mut self, axis: Axis, value: f32) -> Result<()> {
        let mut state = self.state().clone();
        state.axes.insert(axis, value.clamp(0.0, 1.0));
        self.submit_state(state)
    }

    /// Set the canonical six axes (sticks centered at 0.5, triggers 0..1)
    /// and submit.
    pub fn set_standard_axes(&mut self, axes: StandardAxes) -> Result<()> {
        let mut state = self.state().clone();
        state.standard_axes = Some(axes);
        self.submit_state(state)
    }

    /// Remove this controller (hot-unplug). Consumes the handle.
    pub fn remove(self) -> Result<()> {
        self.hm.remove_controller(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_event_buffer_discards_oldest_and_counts_drops() {
        let events = EventBuffer::new(2);
        for controller in ["first", "second", "third"] {
            events.push(OutputEvent {
                controller: controller.into(),
                report_id: 1,
                fields: Default::default(),
                raw: Vec::new(),
                crc_valid: false,
            });
        }

        assert_eq!(events.dropped(), 1);
        assert_eq!(
            events
                .drain()
                .into_iter()
                .map(|event| event.controller)
                .collect::<Vec<_>>(),
            ["second", "third"]
        );
    }

    #[test]
    fn pipe_name_validation_rejects_paths_and_accepts_safe_names() {
        assert_eq!(
            validate_pipe_name("hidmaestro-mcp_1.2").unwrap(),
            "hidmaestro-mcp_1.2"
        );
        for invalid in [
            "",
            ".hidden",
            r"\\.\pipe\hidmaestro",
            "nested/name",
            "name space",
        ] {
            assert!(
                validate_pipe_name(invalid).is_err(),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[test]
    fn filtered_wait_keeps_a_flood_of_unmatched_events_bounded_and_drainable() {
        let events = EventBuffer::new(2);
        for report_id in 0..100 {
            events.push(OutputEvent {
                controller: "other".into(),
                report_id,
                fields: Default::default(),
                raw: Vec::new(),
                crc_valid: false,
            });
        }
        events.push(OutputEvent {
            controller: "target".into(),
            report_id: 100,
            fields: Default::default(),
            raw: Vec::new(),
            crc_valid: false,
        });

        assert_eq!(
            events
                .wait_for_controller("target", Duration::ZERO)
                .unwrap()
                .controller,
            "target"
        );
        assert_eq!(events.dropped(), 99);
        assert_eq!(
            events
                .drain()
                .into_iter()
                .map(|event| event.report_id)
                .collect::<Vec<_>>(),
            [99]
        );
        assert!(events
            .wait_for_controller("target", Duration::ZERO)
            .is_none());

        let timed_out = EventBuffer::new(2);
        for report_id in 0..100 {
            timed_out.push(OutputEvent {
                controller: "other".into(),
                report_id,
                fields: Default::default(),
                raw: Vec::new(),
                crc_valid: false,
            });
        }
        assert!(timed_out
            .wait_for_controller("target", Duration::ZERO)
            .is_none());
        assert_eq!(timed_out.dropped(), 98);
        assert_eq!(timed_out.drain().len(), 2);
    }

    #[test]
    fn bundled_bridge_is_found_next_to_the_current_executable() {
        let directory =
            std::env::temp_dir().join(format!("hidmaestro-bundled-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let current_exe = directory.join("hidmaestro-mcp.exe");
        let bridge = directory.join("hidmaestro-bridge.exe");
        std::fs::write(&bridge, []).unwrap();

        assert_eq!(
            bridge_next_to(&current_exe, "hidmaestro-bridge.exe"),
            Some(bridge)
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn explicit_pipe_name_takes_precedence_over_environment_configuration() {
        let builder = HidMaestro::builder().pipe_name("explicit-pipe");
        assert_eq!(
            builder
                .configured_pipe_name(Some("environment-pipe"))
                .unwrap(),
            Some("explicit-pipe".to_string())
        );
        assert_eq!(
            HidMaestro::builder()
                .configured_pipe_name(Some("environment-pipe"))
                .unwrap(),
            Some("environment-pipe".to_string())
        );
        assert_eq!(
            HidMaestro::builder()
                .configured_pipe_name(Some(""))
                .unwrap(),
            None
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn configured_pipe_mode_is_actionably_unsupported_off_windows() {
        let result = HidMaestro::builder().pipe_name("hidmaestro-mcp").spawn();
        assert!(matches!(result, Err(Error::PipeUnsupported)));
    }
}
