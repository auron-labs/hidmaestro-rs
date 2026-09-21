//! hidmaestro-mcp — MCP stdio server exposing HIDMaestro virtual gamepads
//! to LLM agents.
//!
//! Speaks MCP (JSON-RPC 2.0 over newline-delimited stdio). Requires the
//! `hidmaestro-bridge` executable — point `HIDMAESTRO_BRIDGE_PATH` at it or
//! put it on PATH. The bridge only functions on Windows.

#![recursion_limit = "256"]

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hidmaestro::{Axis, Buttons, Error, GamepadState, Hat, HidMaestro, StandardAxes};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Map, Value};

struct App {
    hm: Option<HidMaestro>,
}

impl App {
    fn hm(&mut self) -> Result<&mut HidMaestro, String> {
        if self.hm.is_none() {
            self.hm =
                Some(HidMaestro::spawn().map_err(|e| format!("failed to start bridge: {e}"))?);
        }
        Ok(self.hm.as_mut().unwrap())
    }

    fn start(&mut self) -> Result<String, String> {
        let reconnect = match self.hm.as_mut() {
            Some(hm) => match hm.ping() {
                Ok(pong) => return Ok(pong),
                Err(Error::BridgeExited(_)) => true,
                Err(error) => return Err(error.to_string()),
            },
            None => false,
        };
        if reconnect {
            self.hm.take();
        }
        self.hm()?.ping().map_err(|error| error.to_string())
    }

    fn state(&mut self, key: &str) -> Result<GamepadState, String> {
        self.hm()?
            .state(key)
            .cloned()
            .ok_or_else(|| format!("no such controller '{key}' (created by this server?)"))
    }

    fn submit_state(&mut self, key: &str, state: &GamepadState) -> Result<(), String> {
        self.hm()?
            .submit_state(key, state)
            .map_err(|e| e.to_string())
    }

    fn update_state(
        &mut self,
        key: &str,
        update: impl FnOnce(&mut GamepadState),
    ) -> Result<(), String> {
        let mut state = self.state(key)?;
        update(&mut state);
        self.submit_state(key, &state)
    }

    fn reset_all_to_neutral(&mut self) -> Result<usize, String> {
        let controllers = self
            .hm()?
            .list_controllers()
            .map_err(|error| error.to_string())?;
        for controller in &controllers {
            self.hm()?
                .submit_state(&controller.key, &GamepadState::neutral())
                .map_err(|error| error.to_string())?;
        }
        Ok(controllers.len())
    }
}

const MAX_ACTIONS: usize = 128;
const MAX_BLOCKING_MS: u64 = 110_000;
const WAIT_SLICE: Duration = Duration::from_millis(50);

fn parse_buttons(v: &Value) -> Result<Buttons, String> {
    match v {
        Value::Array(items) => {
            let mut b = Buttons::NONE;
            for it in items {
                let s = it.as_str().ok_or("buttons must be strings")?;
                b |= Buttons::by_name(s).ok_or_else(|| format!("unknown button '{s}'"))?;
            }
            Ok(b)
        }
        Value::String(s) => Buttons::by_name(s).ok_or_else(|| format!("unknown button '{s}'")),
        Value::Number(n) => {
            let bits = n
                .as_u64()
                .ok_or("buttons bitmask must be an unsigned integer")?;
            let bits = u32::try_from(bits).map_err(|_| "buttons bitmask exceeds u32")?;
            Buttons::from_bits(bits).ok_or_else(|| "buttons bitmask contains unknown bits".into())
        }
        _ => Err("buttons must be a string, array of names, or integer bitmask".into()),
    }
}

fn bounded_ms(value: &Value, name: &str) -> Result<u64, String> {
    let milliseconds = value
        .as_u64()
        .ok_or_else(|| format!("'{name}' must be a non-negative integer"))?;
    if milliseconds > MAX_BLOCKING_MS {
        return Err(format!("'{name}' must be at most {MAX_BLOCKING_MS} ms"));
    }
    Ok(milliseconds)
}

fn optional_axis_value(values: &Map<String, Value>, name: &str) -> Result<Option<f32>, String> {
    let Some(value) = values.get(name) else {
        return Ok(None);
    };
    let value = value
        .as_f64()
        .ok_or_else(|| format!("'{name}' must be a number from 0.0 through 1.0"))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!("'{name}' must be a number from 0.0 through 1.0"));
    }
    Ok(Some(value as f32))
}

fn validate_finite(value: f32, name: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("'{name}' must be a finite number"))
    }
}

fn validate_standard_axes(axes: StandardAxes) -> Result<(), String> {
    for (name, value) in [
        ("left_stick_x", axes.left_stick_x),
        ("left_stick_y", axes.left_stick_y),
        ("right_stick_x", axes.right_stick_x),
        ("right_stick_y", axes.right_stick_y),
        ("left_trigger", axes.left_trigger),
        ("right_trigger", axes.right_trigger),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("standard_axes.{name} must be from 0.0 through 1.0"));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    buttons: Option<Value>,
    hat: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_f32")]
    hat_degrees: Option<Option<f32>>,
    axes: Option<Map<String, Value>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    standard_axes: Option<Option<StandardAxes>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    battery_level: Option<Option<u8>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    battery_charging: Option<Option<bool>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    accel_g: Option<Option<[f32; 3]>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    gyro_dps: Option<Option<[f32; 3]>>,
}

fn deserialize_optional_f32<'de, D>(deserializer: D) -> Result<Option<Option<f32>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Option::<f32>::deserialize(deserializer)?))
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

fn patched_state(mut state: GamepadState, patch: Patch) -> Result<GamepadState, String> {
    if let Some(b) = patch.buttons {
        state.buttons = parse_buttons(&b)?;
    }
    if let Some(h) = patch.hat {
        state.hat = Hat::by_name(&h).ok_or_else(|| format!("unknown direction '{h}'"))?;
        state.hat_degrees = None;
    }
    if let Some(hat_degrees) = patch.hat_degrees {
        if let Some(value) = hat_degrees {
            if !value.is_finite() || !(0.0..360.0).contains(&value) {
                return Err(
                    "hat_degrees must be a finite number from 0.0 (inclusive) to 360.0 (exclusive)"
                        .into(),
                );
            }
        }
        state.hat_degrees = hat_degrees;
    }
    if let Some(axes) = patch.axes {
        for (k, v) in axes {
            let a = Axis::by_name(&k).ok_or_else(|| format!("unknown axis '{k}'"))?;
            let val = v
                .as_f64()
                .ok_or("axis values must be numbers from 0.0 through 1.0")?;
            if !val.is_finite() || !(0.0..=1.0).contains(&val) {
                return Err("axis values must be numbers from 0.0 through 1.0".into());
            }
            state.axes.insert(a, val as f32);
        }
    }
    if let Some(standard_axes) = patch.standard_axes {
        if let Some(axes) = standard_axes {
            validate_standard_axes(axes)?;
        }
        state.standard_axes = standard_axes;
    }
    if let Some(battery_level) = patch.battery_level {
        if let Some(level) = battery_level {
            if level > 10 {
                return Err("battery_level must be an integer from 0 through 10".into());
            }
        }
        state.battery_level = battery_level;
    }
    if let Some(battery_charging) = patch.battery_charging {
        state.battery_charging = battery_charging;
    }
    if let Some(accel_g) = patch.accel_g {
        if let Some(values) = accel_g {
            for value in values {
                validate_finite(value, "accel_g values")?;
            }
        }
        state.accel_g = accel_g;
    }
    if let Some(gyro_dps) = patch.gyro_dps {
        if let Some(values) = gyro_dps {
            for value in values {
                validate_finite(value, "gyro_dps values")?;
            }
        }
        state.gyro_dps = gyro_dps;
    }
    Ok(state)
}

fn tool_error(msg: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": msg.into()}], "isError": true})
}

fn tool_ok(text: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}]})
}

fn tool_json<T: serde::Serialize>(v: &T) -> Value {
    let value = serde_json::to_value(v).unwrap_or(Value::Null);
    let structured_content = match &value {
        Value::Object(_) => value.clone(),
        _ => json!({"items": value}),
    };
    json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default()}],
        "structuredContent": structured_content
    })
}

static APP: Mutex<Option<App>> = Mutex::new(None);

fn with_app<R>(f: impl FnOnce(&mut App) -> Result<R, String>) -> Result<R, String> {
    let mut g = APP.lock().map_err(|_| "server lock poisoned".to_string())?;
    if g.is_none() {
        *g = Some(App { hm: None });
    }
    f(g.as_mut().unwrap())
}

fn sleep_until(target: Instant) {
    while let Some(remaining) = target.checked_duration_since(Instant::now()) {
        if remaining.is_zero() {
            return;
        }
        std::thread::sleep(remaining.min(WAIT_SLICE));
    }
}

fn apply_axis_patch(state: &mut GamepadState, axes: Map<String, Value>) -> Result<(), String> {
    for (name, value) in axes {
        let axis = Axis::by_name(&name).ok_or_else(|| format!("unknown axis '{name}'"))?;
        let value = value
            .as_f64()
            .ok_or_else(|| format!("axis '{name}' must be a number from 0.0 through 1.0"))?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(format!(
                "axis '{name}' must be a number from 0.0 through 1.0"
            ));
        }
        state.axes.insert(axis, value as f32);
    }
    Ok(())
}

struct SequenceAction {
    at_ms: u64,
    state: Option<Patch>,
    press_buttons: Option<Buttons>,
    release_buttons: Option<Buttons>,
    dpad: Option<Hat>,
    axes: Option<Map<String, Value>>,
}

fn parse_sequence_actions(args: &Map<String, Value>) -> Result<Vec<SequenceAction>, String> {
    let actions = args
        .get("actions")
        .and_then(Value::as_array)
        .ok_or("'actions' must be a non-empty array")?;
    if actions.is_empty() || actions.len() > MAX_ACTIONS {
        return Err(format!(
            "'actions' must contain from 1 through {MAX_ACTIONS} entries"
        ));
    }

    let mut previous_offset = 0;
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let object = action
                .as_object()
                .ok_or_else(|| format!("actions[{index}] must be an object"))?;
            if object.keys().any(|key| {
                ![
                    "at_ms",
                    "state",
                    "press_buttons",
                    "release_buttons",
                    "dpad",
                    "axes",
                ]
                .contains(&key.as_str())
            }) {
                return Err(format!("actions[{index}] contains an unknown field"));
            }
            let at_ms = bounded_ms(
                object
                    .get("at_ms")
                    .ok_or_else(|| format!("actions[{index}] is missing 'at_ms'"))?,
                "at_ms",
            )?;
            if at_ms < previous_offset {
                return Err("action offsets must be non-decreasing relative milliseconds".into());
            }
            previous_offset = at_ms;

            let state = match object.get("state") {
                Some(value) if value.is_object() => Some(
                    serde_json::from_value::<Patch>(value.clone())
                        .map_err(|error| format!("invalid actions[{index}].state: {error}"))?,
                ),
                Some(_) => return Err(format!("actions[{index}].state must be an object")),
                None => None,
            };
            let press_buttons = object
                .get("press_buttons")
                .map(parse_buttons)
                .transpose()
                .map_err(|error| format!("invalid actions[{index}].press_buttons: {error}"))?;
            let release_buttons = object
                .get("release_buttons")
                .map(parse_buttons)
                .transpose()
                .map_err(|error| format!("invalid actions[{index}].release_buttons: {error}"))?;
            let dpad = match object.get("dpad") {
                Some(Value::String(direction)) => Hat::by_name(direction).ok_or_else(|| {
                    format!("unknown actions[{index}].dpad direction '{direction}'")
                })?,
                Some(_) => return Err(format!("actions[{index}].dpad must be a direction string")),
                None => Hat::None,
            };
            let dpad = object.get("dpad").map(|_| dpad);
            let axes = match object.get("axes") {
                Some(Value::Object(axes)) => Some(axes.clone()),
                Some(_) => return Err(format!("actions[{index}].axes must be an object")),
                None => None,
            };
            if let Some(patch) = &state {
                patched_state(GamepadState::neutral(), patch.clone())
                    .map_err(|error| format!("invalid actions[{index}].state: {error}"))?;
            }
            if let Some(axes) = &axes {
                let mut neutral = GamepadState::neutral();
                apply_axis_patch(&mut neutral, axes.clone())
                    .map_err(|error| format!("invalid actions[{index}].axes: {error}"))?;
            }
            if state.is_none()
                && press_buttons.is_none()
                && release_buttons.is_none()
                && dpad.is_none()
                && axes.is_none()
            {
                return Err(format!("actions[{index}] must include a state change"));
            }
            Ok(SequenceAction {
                at_ms,
                state,
                press_buttons,
                release_buttons,
                dpad,
                axes,
            })
        })
        .collect()
}

fn run_sequence(app: &mut App, key: &str, actions: Vec<SequenceAction>) -> Result<(), String> {
    let started = Instant::now();
    for action in actions {
        sleep_until(started + Duration::from_millis(action.at_ms));
        let state = app
            .hm()?
            .state(key)
            .cloned()
            .ok_or_else(|| format!("no such controller '{key}'"))?;
        let state = apply_sequence_action(state, action)?;
        app.hm()?
            .submit_state(key, &state)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn apply_sequence_action(
    mut state: GamepadState,
    action: SequenceAction,
) -> Result<GamepadState, String> {
    if let Some(patch) = action.state {
        state = patched_state(state, patch)?;
    }
    if let Some(buttons) = action.press_buttons {
        state.buttons |= buttons;
    }
    if let Some(buttons) = action.release_buttons {
        state.buttons -= buttons;
    }
    if let Some(dpad) = action.dpad {
        state.hat = dpad;
        state.hat_degrees = None;
    }
    if let Some(axes) = action.axes {
        apply_axis_patch(&mut state, axes)?;
    }
    Ok(state)
}

fn wait_for_output_event(
    app: &mut App,
    timeout: Duration,
    controller: Option<&str>,
) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let hm = app.hm()?;
        let event = match controller {
            Some(controller) => hm.wait_event_for_controller(controller, remaining.min(WAIT_SLICE)),
            None => hm.wait_event(remaining.min(WAIT_SLICE)),
        }
        .map_err(|e| e.to_string())?;
        match event {
            Some(event) => {
                return Ok(tool_json(&json!({
                    "events": [event],
                    "dropped_event_count": hm.dropped_event_count()
                })));
            }
            None if Instant::now() >= deadline => {
                return Ok(tool_json(&json!({
                    "events": [],
                    "dropped_event_count": hm.dropped_event_count()
                })));
            }
            None => {}
        }
    }
}

fn call_tool(name: &str, args: Map<String, Value>) -> Result<Value, String> {
    match name {
        // ── lifecycle ────────────────────────────────────────────────
        "start" => with_app(|app| {
            let pong = app.start()?;
            Ok(tool_ok(format!("bridge running ({pong})")))
        }),
        "status" => with_app(|app| {
            let hm = app.hm()?;
            let installed = hm.is_driver_installed().map_err(|e| e.to_string())?;
            let controllers = hm.list_controllers().map_err(|e| e.to_string())?;
            Ok(tool_json(&json!({
                "driver_installed": installed,
                "controllers": controllers,
            })))
        }),
        "install_driver" => with_app(|app| {
            app.hm()?.install_driver().map_err(|e| e.to_string())?;
            Ok(tool_ok("driver installed (or already present)"))
        }),
        "install_usbip_backend" => with_app(|app| {
            app.hm()?
                .install_usbip_backend()
                .map_err(|e| e.to_string())?;
            Ok(tool_ok("usbip backend installed"))
        }),
        "load_profiles" => with_app(|app| {
            let hm = app.hm()?;
            let n = match args.get("dir").and_then(|d| d.as_str()) {
                Some(dir) => hm.load_profiles_from_dir(dir).map_err(|e| e.to_string())?,
                None => hm.load_default_profiles().map_err(|e| e.to_string())?,
            };
            Ok(tool_ok(format!("{n} profiles loaded")))
        }),
        "shutdown" => with_app(|app| {
            if let Some(hm) = app.hm.take() {
                hm.shutdown();
            }
            Ok(tool_ok("bridge stopped; all virtual controllers removed"))
        }),

        // ── profiles & controllers ───────────────────────────────────
        "list_profiles" => with_app(|app| {
            let filter = args
                .get("filter")
                .and_then(|f| f.as_str())
                .map(|f| f.to_lowercase());
            let mut profiles = app.hm()?.list_profiles().map_err(|e| e.to_string())?;
            if let Some(f) = filter {
                profiles.retain(|p| {
                    p.id.to_lowercase().contains(&f)
                        || p.name.to_lowercase().contains(&f)
                        || p.vendor.to_lowercase().contains(&f)
                });
            }
            Ok(tool_json(&profiles))
        }),
        "get_profile" => with_app(|app| {
            let profile_id = args
                .get("profile_id")
                .and_then(Value::as_str)
                .ok_or("missing 'profile_id'")?;
            let profile = app
                .hm()?
                .get_profile(profile_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("profile '{profile_id}' not found"))?;
            Ok(tool_json(&profile))
        }),
        "create_controller" => with_app(|app| {
            let profile_id = args
                .get("profile_id")
                .and_then(|p| p.as_str())
                .ok_or("missing 'profile_id'")?;
            let index = args
                .get("index")
                .map(|value| {
                    value
                        .as_u64()
                        .ok_or("'index' must be a non-negative integer")
                        .and_then(|index| {
                            usize::try_from(index)
                                .map_err(|_| "'index' is too large for this platform")
                        })
                })
                .transpose()?;
            let key = app
                .hm()?
                .create_controller(profile_id, index)
                .map_err(|e| e.to_string())?;
            Ok(tool_ok(format!(
                "controller '{key}' created from profile '{profile_id}'"
            )))
        }),
        "list_controllers" => with_app(|app| {
            let list = app.hm()?.list_controllers().map_err(|e| e.to_string())?;
            Ok(tool_json(&list))
        }),
        "remove_controller" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            app.hm()?
                .remove_controller(key)
                .map_err(|e| e.to_string())?;
            Ok(tool_ok(format!("controller '{key}' removed")))
        }),
        "remove_all_controllers" => with_app(|app| {
            app.hm()?
                .remove_all_controllers()
                .map_err(|e| e.to_string())?;
            Ok(tool_ok("all virtual controllers removed"))
        }),

        // ── input ────────────────────────────────────────────────────
        "press_buttons" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let buttons = parse_buttons(args.get("buttons").ok_or("missing 'buttons'")?)?;
            let hold = args
                .get("hold_ms")
                .map(|value| bounded_ms(value, "hold_ms"))
                .transpose()?;
            app.update_state(key, |state| state.buttons |= buttons)?;
            if let Some(hold) = hold {
                sleep_until(Instant::now() + Duration::from_millis(hold));
                app.update_state(key, |state| state.buttons -= buttons)?;
                Ok(tool_ok(format!(
                    "tapped {} on '{key}' ({hold} ms)",
                    buttons.names().join("+")
                )))
            } else {
                Ok(tool_ok(format!(
                    "holding {} on '{key}'",
                    buttons.names().join("+")
                )))
            }
        }),
        "hold_buttons" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(Value::as_str)
                .ok_or("missing 'controller'")?;
            let buttons = parse_buttons(args.get("buttons").ok_or("missing 'buttons'")?)?;
            app.update_state(key, |state| state.buttons |= buttons)?;
            Ok(tool_ok(format!(
                "holding {} on '{key}' until release_buttons or reset_controller",
                buttons.names().join("+")
            )))
        }),
        "tap_buttons" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(Value::as_str)
                .ok_or("missing 'controller'")?;
            let buttons = parse_buttons(args.get("buttons").ok_or("missing 'buttons'")?)?;
            let duration = bounded_ms(
                args.get("duration_ms").ok_or("missing 'duration_ms'")?,
                "duration_ms",
            )?;
            app.update_state(key, |state| state.buttons |= buttons)?;
            sleep_until(Instant::now() + Duration::from_millis(duration));
            app.update_state(key, |state| state.buttons -= buttons)?;
            Ok(tool_ok(format!(
                "tapped {} on '{key}' ({duration} ms)",
                buttons.names().join("+")
            )))
        }),
        "release_buttons" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let buttons = parse_buttons(args.get("buttons").ok_or("missing 'buttons'")?)?;
            app.update_state(key, |state| state.buttons -= buttons)?;
            Ok(tool_ok(format!(
                "released {} on '{key}'",
                buttons.names().join("+")
            )))
        }),
        "set_buttons" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let buttons = parse_buttons(args.get("buttons").ok_or("missing 'buttons'")?)?;
            app.update_state(key, |state| state.buttons = buttons)?;
            Ok(tool_ok(format!(
                "'{key}' buttons = [{}]",
                buttons.names().join(", ")
            )))
        }),
        "set_axis" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let axis_name = args
                .get("axis")
                .and_then(|a| a.as_str())
                .ok_or("missing 'axis'")?;
            let value = optional_axis_value(&args, "value")?.ok_or("missing 'value'")?;
            let axis =
                Axis::by_name(axis_name).ok_or_else(|| format!("unknown axis '{axis_name}'"))?;
            app.update_state(key, |state| {
                state.axes.insert(axis, value);
            })?;
            Ok(tool_ok(format!("'{key}' axis {axis_name} = {value:.3}")))
        }),
        "set_sticks" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let left_x = optional_axis_value(&args, "left_x")?;
            let left_y = optional_axis_value(&args, "left_y")?;
            let right_x = optional_axis_value(&args, "right_x")?;
            let right_y = optional_axis_value(&args, "right_y")?;
            let left_trigger = optional_axis_value(&args, "left_trigger")?;
            let right_trigger = optional_axis_value(&args, "right_trigger")?;
            app.update_state(key, |state| {
                let mut axes = state.standard_axes.unwrap_or_default();
                if let Some(v) = left_x {
                    axes.left_stick_x = Some(v);
                }
                if let Some(v) = left_y {
                    axes.left_stick_y = Some(v);
                }
                if let Some(v) = right_x {
                    axes.right_stick_x = Some(v);
                }
                if let Some(v) = right_y {
                    axes.right_stick_y = Some(v);
                }
                if let Some(v) = left_trigger {
                    axes.left_trigger = Some(v);
                }
                if let Some(v) = right_trigger {
                    axes.right_trigger = Some(v);
                }
                state.standard_axes = Some(axes);
            })?;
            Ok(tool_ok(format!("'{key}' sticks/triggers updated")))
        }),
        "set_dpad" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let dir = args
                .get("direction")
                .and_then(|d| d.as_str())
                .ok_or("missing 'direction'")?;
            let hat = Hat::by_name(dir).ok_or_else(|| format!("unknown direction '{dir}'"))?;
            app.update_state(key, |state| {
                state.hat = hat;
                state.hat_degrees = None;
            })?;
            Ok(tool_ok(format!("'{key}' d-pad = {dir}")))
        }),
        "set_state" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let raw = args
                .get("state")
                .cloned()
                .unwrap_or(Value::Object(Map::new()));
            let patch: Patch =
                serde_json::from_value(raw).map_err(|e| format!("invalid state: {e}"))?;
            let state = app.state(key)?;
            let state = patched_state(state, patch)?;
            app.submit_state(key, &state)?;
            Ok(tool_ok(format!("'{key}' state updated")))
        }),
        "get_state" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            Ok(tool_json(&app.state(key)?))
        }),
        "reset_controller" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            app.update_state(key, |state| *state = GamepadState::neutral())?;
            Ok(tool_ok(format!("'{key}' reset to neutral")))
        }),
        "run_action_sequence" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(Value::as_str)
                .ok_or("missing 'controller'")?;
            let actions = parse_sequence_actions(&args)?;
            run_sequence(app, key, actions)?;
            Ok(tool_ok(format!("action sequence completed on '{key}'")))
        }),
        "drain_output_events" => with_app(|app| {
            let events = app.hm()?.drain_events();
            let dropped_event_count = app.hm()?.dropped_event_count();
            Ok(tool_json(
                &json!({"events": events, "dropped_event_count": dropped_event_count}),
            ))
        }),
        "wait_for_output_events" => with_app(|app| {
            let timeout_ms = bounded_ms(
                args.get("timeout_ms").ok_or("missing 'timeout_ms'")?,
                "timeout_ms",
            )?;
            let controller = match args.get("controller") {
                Some(Value::String(controller)) => Some(controller.as_str()),
                Some(_) => return Err("'controller' must be a string".into()),
                None => None,
            };
            wait_for_output_event(app, Duration::from_millis(timeout_ms), controller)
        }),

        _ => Err(format!("unknown tool '{name}'")),
    }
}

fn state_patch_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]},
            "hat": {"type": "string"},
            "hat_degrees": {"type": ["number", "null"], "minimum": 0.0, "exclusiveMaximum": 360.0},
            "axes": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0.0, "maximum": 1.0}},
            "standard_axes": {"type": ["object", "null"], "additionalProperties": false, "properties": {"left_stick_x": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "left_stick_y": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "right_stick_x": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "right_stick_y": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "left_trigger": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "right_trigger": {"type": "number", "minimum": 0.0, "maximum": 1.0}}},
            "battery_level": {"type": ["integer", "null"], "minimum": 0, "maximum": 10},
            "battery_charging": {"type": ["boolean", "null"]},
            "accel_g": {"type": ["array", "null"], "minItems": 3, "maxItems": 3, "items": {"type": "number"}},
            "gyro_dps": {"type": ["array", "null"], "minItems": 3, "maxItems": 3, "items": {"type": "number"}}
        }
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "start",
            "description": "Start the hidmaestro bridge process (usually automatic).",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "status",
            "description": "Bridge/driver status and live controllers.",
            "inputSchema": {"type": "object", "properties": {}},
            "outputSchema": {"type": "object", "required": ["driver_installed", "controllers"], "properties": {"driver_installed": {"type": "boolean"}, "controllers": {"type": "array"}}}
        },
        {
            "name": "install_driver",
            "description": "Install the HIDMaestro UMDF2 driver. Requires an elevated host the first time.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "install_usbip_backend",
            "description": "Install the usbip-win2 backend needed by '-composite' profiles.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "load_profiles",
            "description": "Load the profile catalog (or a directory of profile JSONs).",
            "inputSchema": {"type": "object", "properties": {
                "dir": {"type": "string", "description": "Optional directory of profile JSON files"}
            }}
        },
        {
            "name": "list_profiles",
            "description": "List loaded device profiles (virtual controller models).",
            "inputSchema": {"type": "object", "properties": {
                "filter": {"type": "string", "description": "Substring filter on id/name/vendor"}
            }},
            "outputSchema": {"type": "object", "required": ["items"], "properties": {"items": {"type": "array", "items": {"type": "object"}}}}
        },
        {
            "name": "get_profile",
            "description": "Inspect one loaded profile, including only upstream-reported capability metadata such as button_count, axis_count, hat support, backend, and deployment requirements.",
            "inputSchema": {"type": "object", "required": ["profile_id"], "properties": {"profile_id": {"type": "string"}}},
            "outputSchema": {"type": "object", "required": ["id", "button_count", "axis_count", "has_hat"], "properties": {"id": {"type": "string"}, "button_count": {"type": "integer"}, "axis_count": {"type": "integer"}, "has_hat": {"type": "boolean"}, "deployable": {"type": "boolean"}, "requires_usbip_backend": {"type": "boolean"}}}
        },
        {
            "name": "create_controller",
            "description": "Create a virtual controller from a profile id (e.g. 'xbox-360-wired', 'dualsense').",
            "inputSchema": {"type": "object", "required": ["profile_id"], "properties": {
                "profile_id": {"type": "string"},
                "index": {"type": "integer", "description": "Optional enumeration slot"}
            }}
        },
        {
            "name": "list_controllers",
            "description": "List live virtual controllers and their keys.",
            "inputSchema": {"type": "object", "properties": {}},
            "outputSchema": {"type": "object", "required": ["items"], "properties": {"items": {"type": "array", "items": {"type": "object"}}}}
        },
        {
            "name": "remove_controller",
            "description": "Hot-unplug a virtual controller.",
            "inputSchema": {"type": "object", "required": ["controller"], "properties": {
                "controller": {"type": "string", "description": "Controller key, e.g. 'c1'"}
            }}
        },
        {
            "name": "remove_all_controllers",
            "description": "Remove every virtual controller, including strays from previous sessions.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "press_buttons",
            "description": "Legacy-compatible hold operation; with hold_ms, it taps instead. Prefer hold_buttons or tap_buttons for explicit semantics. Names: a,b,x,y,left_bumper,right_bumper,back,start,left_stick,right_stick,guide,touchpad,share,misc1 (PlayStation aliases cross,circle,square,triangle work too).",
            "inputSchema": {"type": "object", "required": ["controller", "buttons"], "properties": {
                "controller": {"type": "string"},
                "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]},
                "hold_ms": {"type": "integer", "minimum": 0, "maximum": 110000, "description": "If set, press then release after this many ms"}
            }}
        },
        {
            "name": "hold_buttons",
            "description": "Explicitly hold buttons until release_buttons or reset_controller. This tool never sleeps.",
            "inputSchema": {"type": "object", "required": ["controller", "buttons"], "properties": {"controller": {"type": "string"}, "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]}}}
        },
        {
            "name": "tap_buttons",
            "description": "Press buttons and reliably submit their release after duration_ms (0 through 110000 ms).",
            "inputSchema": {"type": "object", "required": ["controller", "buttons", "duration_ms"], "properties": {"controller": {"type": "string"}, "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]}, "duration_ms": {"type": "integer", "minimum": 0, "maximum": 110000}}}
        },
        {
            "name": "release_buttons",
            "description": "Release held buttons.",
            "inputSchema": {"type": "object", "required": ["controller", "buttons"], "properties": {
                "controller": {"type": "string"},
                "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]}
            }}
        },
        {
            "name": "set_buttons",
            "description": "Replace the entire pressed-button set.",
            "inputSchema": {"type": "object", "required": ["controller", "buttons"], "properties": {
                "controller": {"type": "string"},
                "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]}
            }}
        },
        {
            "name": "set_axis",
            "description": "Set one analog axis by name (0.0..1.0; 0.5 = stick center, 0.0 = trigger released). Names: x,y,z,rx,ry,rz,slider,dial,wheel,throttle,brake,clutch,rudder, or a raw usage like '0x0130'.",
            "inputSchema": {"type": "object", "required": ["controller", "axis", "value"], "properties": {
                "controller": {"type": "string"},
                "axis": {"type": "string"},
                "value": {"type": "number", "minimum": 0.0, "maximum": 1.0}
            }}
        },
        {
            "name": "set_sticks",
            "description": "Set canonical sticks/triggers (each 0.0..1.0; omitted fields unchanged).",
            "inputSchema": {"type": "object", "required": ["controller"], "properties": {
                "controller": {"type": "string"},
                "left_x": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "left_y": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                "right_x": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "right_y": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                "left_trigger": {"type": "number", "minimum": 0.0, "maximum": 1.0}, "right_trigger": {"type": "number", "minimum": 0.0, "maximum": 1.0}
            }}
        },
        {
            "name": "set_dpad",
            "description": "Set the d-pad: none, up, up_right, right, down_right, down, down_left, left, up_left.",
            "inputSchema": {"type": "object", "required": ["controller", "direction"], "properties": {
                "controller": {"type": "string"},
                "direction": {"type": "string"}
            }}
        },
        {
            "name": "set_state",
            "description": "Patch several state fields at once. Analog axes and standard_axes must be 0.0 through 1.0; hat_degrees is 0.0 inclusive through 360.0 exclusive; battery_level is 0 through 10; accel_g and gyro_dps are finite triples.",
            "inputSchema": {"type": "object", "required": ["controller", "state"], "properties": {
                "controller": {"type": "string"},
                "state": state_patch_schema()
            }}
        },
        {
            "name": "get_state",
            "description": "Return the server's mirrored state for a controller.",
            "inputSchema": {"type": "object", "required": ["controller"], "properties": {
                "controller": {"type": "string"}
            }},
            "outputSchema": {"type": "object", "properties": {"buttons": {"type": "integer"}, "hat": {"type": "integer"}, "axes": {"type": "object"}}}
        },
        {
            "name": "reset_controller",
            "description": "Reset a controller to neutral (sticks centered, nothing held).",
            "inputSchema": {"type": "object", "required": ["controller"], "properties": {
                "controller": {"type": "string"}
            }}
        },
        {
            "name": "drain_output_events",
            "description": "Drain raw and decoded output reports (rumble/LED/FFB writes games sent to the virtual pad).",
            "inputSchema": {"type": "object", "properties": {}},
            "outputSchema": {"type": "object", "required": ["events", "dropped_event_count"], "properties": {"events": {"type": "array", "items": {"type": "object"}}, "dropped_event_count": {"type": "integer", "minimum": 0}}}
        },
        {
            "name": "wait_for_output_events",
            "description": "Wait up to timeout_ms (0 through 110000) for one output event, optionally from one controller. Unmatched events are preserved for drain_output_events. Cancellation notifications receive no response, but this synchronous server cannot interrupt an active wait; waiting uses 50 ms increments.",
            "inputSchema": {"type": "object", "required": ["timeout_ms"], "properties": {"timeout_ms": {"type": "integer", "minimum": 0, "maximum": 110000}, "controller": {"type": "string"}}},
            "outputSchema": {"type": "object", "required": ["events", "dropped_event_count"], "properties": {"events": {"type": "array", "maxItems": 1, "items": {"type": "object"}}, "dropped_event_count": {"type": "integer", "minimum": 0}}}
        },
        {
            "name": "run_action_sequence",
            "description": "Apply up to 128 ordered controller changes at non-decreasing relative at_ms offsets (0 through 110000). Each action applies state first, then press_buttons/release_buttons, dpad, and axes. The sequence uses 50 ms sleep increments; cancellation notifications cannot interrupt an active synchronous sequence.",
            "inputSchema": {"type": "object", "required": ["controller", "actions"], "properties": {
                "controller": {"type": "string"},
                "actions": {"type": "array", "minItems": 1, "maxItems": 128, "items": {"type": "object", "required": ["at_ms"], "additionalProperties": false, "properties": {
                    "at_ms": {"type": "integer", "minimum": 0, "maximum": 110000},
                    "press_buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]},
                    "release_buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]},
                    "dpad": {"type": "string"},
                    "axes": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0.0, "maximum": 1.0}},
                    "state": state_patch_schema()
                }}}
            }}
        },
        {
            "name": "shutdown",
            "description": "Stop the bridge and remove all virtual controllers.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

const LEGACY_PROTOCOL_VERSION: &str = "2025-03-26";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
const INACTIVITY_ENV: &str = "HIDMAESTRO_MCP_INACTIVITY_MS";
const MODERN_PROTOCOL_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const MODERN_CLIENT_CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const MODERN_SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";

fn rpc_error(msg: &Value, code: i64, message: impl Into<String>) -> Option<Value> {
    msg.get("id").cloned().map(|id| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message.into()}
        })
    })
}

fn rpc_error_with_data(
    msg: &Value,
    code: i64,
    message: impl Into<String>,
    data: Value,
) -> Option<Value> {
    msg.get("id").cloned().map(|id| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message.into(), "data": data}
        })
    })
}

fn server_info() -> Value {
    json!({"name": "hidmaestro-mcp", "version": env!("CARGO_PKG_VERSION")})
}

fn legacy_server_description(protocol_version: &str) -> Result<Value, String> {
    if protocol_version != LEGACY_PROTOCOL_VERSION {
        return Err(format!(
            "unsupported legacy protocol version '{protocol_version}'; supported version is {LEGACY_PROTOCOL_VERSION}"
        ));
    }
    Ok(json!({
        "protocolVersion": LEGACY_PROTOCOL_VERSION,
        "capabilities": {"tools": {}},
        "serverInfo": server_info()
    }))
}

fn modern_discovery_result() -> Value {
    json!({
        "supportedVersions": [MODERN_PROTOCOL_VERSION],
        "capabilities": {"tools": {"listChanged": false}}
    })
}

fn modern_result(mut result: Value) -> Value {
    let result = result
        .as_object_mut()
        .expect("all MCP results produced by this server are objects");
    result.insert("resultType".into(), Value::String("complete".into()));
    result.insert(
        "_meta".into(),
        json!({MODERN_SERVER_INFO_KEY: server_info()}),
    );
    Value::Object(result.clone())
}

fn requested_protocol<'a>(msg: &'a Value, default: &'static str) -> Result<&'a str, String> {
    let Some(params) = msg.get("params") else {
        return Ok(default);
    };
    let params = params
        .as_object()
        .ok_or("request params must be an object")?;
    match params.get("protocolVersion") {
        Some(Value::String(version)) => Ok(version),
        Some(_) => Err("'protocolVersion' must be a string".into()),
        None => Ok(default),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionEra {
    Legacy,
    Modern,
}

enum ModernMetadataError {
    Invalid(String),
    UnsupportedVersion(String),
}

fn modern_metadata(msg: &Value, required: bool) -> Result<bool, ModernMetadataError> {
    let Some(params) = msg.get("params") else {
        return if required {
            Err(ModernMetadataError::Invalid(
                "modern requests require params._meta".into(),
            ))
        } else {
            Ok(false)
        };
    };
    let params = params.as_object().ok_or_else(|| {
        ModernMetadataError::Invalid("modern request params must be an object".into())
    })?;
    let Some(meta) = params.get("_meta") else {
        return if required {
            Err(ModernMetadataError::Invalid(
                "modern requests require params._meta".into(),
            ))
        } else {
            Ok(false)
        };
    };
    let meta = meta.as_object().ok_or_else(|| {
        ModernMetadataError::Invalid("modern request params._meta must be an object".into())
    })?;
    let has_modern_key =
        meta.contains_key(MODERN_PROTOCOL_KEY) || meta.contains_key(MODERN_CLIENT_CAPABILITIES_KEY);
    if !required && !has_modern_key {
        return Ok(false);
    }
    let requested = meta
        .get(MODERN_PROTOCOL_KEY)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ModernMetadataError::Invalid(format!(
                "modern requests require params._meta['{MODERN_PROTOCOL_KEY}']"
            ))
        })?;
    if !meta
        .get(MODERN_CLIENT_CAPABILITIES_KEY)
        .is_some_and(Value::is_object)
    {
        return Err(ModernMetadataError::Invalid(format!(
            "modern requests require params._meta['{MODERN_CLIENT_CAPABILITIES_KEY}'] as an object"
        )));
    }
    if requested != MODERN_PROTOCOL_VERSION {
        return Err(ModernMetadataError::UnsupportedVersion(requested.into()));
    }
    Ok(true)
}

fn modern_metadata_error(msg: &Value, error: ModernMetadataError) -> Option<Value> {
    match error {
        ModernMetadataError::Invalid(message) => rpc_error(msg, -32602, message),
        ModernMetadataError::UnsupportedVersion(requested) => rpc_error_with_data(
            msg,
            -32022,
            format!("unsupported protocol version '{requested}'"),
            json!({"supported": [MODERN_PROTOCOL_VERSION], "requested": requested}),
        ),
    }
}

fn tool_exists(name: &str) -> bool {
    tools()
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
}

fn parse_tool_call(msg: &Value) -> Result<(&str, Map<String, Value>), String> {
    let params = msg
        .get("params")
        .and_then(Value::as_object)
        .ok_or("tools/call params must be an object")?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or("tools/call requires a non-empty string 'name'")?;
    if !tool_exists(name) {
        return Err(format!("unknown tool '{name}'"));
    }
    let arguments = match params.get("arguments") {
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => return Err("tools/call 'arguments' must be an object".into()),
        None => Map::new(),
    };
    Ok((name, arguments))
}

fn handle_with_session(msg: &Value, session: &mut Option<SessionEra>) -> Option<Value> {
    if msg.get("jsonrpc") != Some(&Value::String("2.0".into())) {
        return rpc_error(msg, -32600, "Invalid Request: jsonrpc must be '2.0'");
    }
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return rpc_error(msg, -32600, "Invalid Request: method must be a string");
    };

    if method == "initialize" {
        let result = match requested_protocol(msg, LEGACY_PROTOCOL_VERSION)
            .and_then(legacy_server_description)
        {
            Ok(result) => result,
            Err(error) => return rpc_error(msg, -32602, error),
        };
        *session = Some(SessionEra::Legacy);
        return msg
            .get("id")
            .cloned()
            .map(|id| json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    let modern_required = method == "server/discover" || *session == Some(SessionEra::Modern);
    let modern = match modern_metadata(msg, modern_required) {
        Ok(modern) => modern,
        Err(error) => return modern_metadata_error(msg, error),
    };

    let result = match method {
        "server/discover" => {
            *session = Some(SessionEra::Modern);
            modern_discovery_result()
        }
        "notifications/initialized" | "notifications/cancelled" => return None,
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let (name, args) = match parse_tool_call(msg) {
                Ok(call) => call,
                Err(error) => return rpc_error(msg, -32602, error),
            };
            match call_tool(name, args) {
                Ok(result) => result,
                Err(error) => tool_error(error),
            }
        }
        _ => return rpc_error(msg, -32601, "Method not found"),
    };
    let result = if modern {
        modern_result(result)
    } else {
        result
    };

    msg.get("id")
        .cloned()
        .map(|id| json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

#[cfg(test)]
fn handle(msg: &Value) -> Option<Value> {
    handle_with_session(msg, &mut None)
}

fn inactivity_timeout() -> Option<Duration> {
    parse_inactivity_timeout(std::env::var(INACTIVITY_ENV).ok().as_deref())
}

fn parse_inactivity_timeout(value: Option<&str>) -> Option<Duration> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|milliseconds| *milliseconds > 0)
        .map(Duration::from_millis)
}

fn reset_for_inactivity() {
    let Ok(mut app) = APP.lock() else {
        return;
    };
    if let Some(app) = app.as_mut().filter(|app| app.hm.is_some()) {
        let _ = app.reset_all_to_neutral();
    }
}

fn shutdown_app() {
    let Ok(mut app) = APP.lock() else {
        return;
    };
    if let Some(App { hm: Some(hm) }) = app.take() {
        hm.shutdown();
    }
}

fn main() {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in BufReader::new(stdin.lock()).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let inactivity = inactivity_timeout();
    let mut stdout = std::io::stdout();
    let mut watchdog_fired = false;
    let mut session = None;
    loop {
        let line = match inactivity {
            Some(timeout) => match receiver.recv_timeout(timeout) {
                Ok(line) => {
                    watchdog_fired = false;
                    line
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !watchdog_fired {
                        reset_for_inactivity();
                        watchdog_fired = true;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match receiver.recv() {
                Ok(line) => line,
                Err(_) => break,
            },
        };
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}})
                );
                let _ = stdout.flush();
                continue;
            }
        };
        if let Some(response) = handle_with_session(&msg, &mut session) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
    shutdown_app();
}

#[cfg(test)]
mod set_state_atomic_tests {
    use super::*;

    #[test]
    fn rejected_patch_leaves_cached_state_unchanged() {
        let mut state = GamepadState::neutral();
        state.buttons = Buttons::A;
        let before = state.clone();
        let patch: Patch = serde_json::from_value(json!({
            "buttons": "b",
            "axes": {"not_an_axis": 0.5}
        }))
        .unwrap();

        assert!(patched_state(state.clone(), patch).is_err());
        assert_eq!(state, before);
    }
}

#[cfg(test)]
mod hat_patch_tests {
    use super::*;

    fn state_with_angle() -> GamepadState {
        let mut state = GamepadState::neutral();
        state.hat_degrees = Some(45.0);
        state
    }

    #[test]
    fn discrete_hat_clears_degrees() {
        for (hat, expected) in [("none", Hat::None), ("north", Hat::North)] {
            let patch = serde_json::from_value(json!({"hat": hat})).unwrap();
            let state = patched_state(state_with_angle(), patch).unwrap();

            assert_eq!(state.hat, expected);
            assert_eq!(state.hat_degrees, None);
        }
    }

    #[test]
    fn omitted_hat_fields_preserve_degrees() {
        let patch = serde_json::from_value(json!({"buttons": "a"})).unwrap();

        assert_eq!(
            patched_state(state_with_angle(), patch)
                .unwrap()
                .hat_degrees,
            Some(45.0)
        );
    }

    #[test]
    fn null_hat_degrees_clears_angle() {
        let patch = serde_json::from_value(json!({"hat_degrees": null})).unwrap();

        assert_eq!(
            patched_state(state_with_angle(), patch)
                .unwrap()
                .hat_degrees,
            None
        );
    }

    #[test]
    fn numeric_angle_overrides_discrete_hat_when_both_are_set() {
        let patch = serde_json::from_value(json!({"hat": "east", "hat_degrees": 270.0})).unwrap();
        let state = patched_state(state_with_angle(), patch).unwrap();

        assert_eq!(state.hat, Hat::East);
        assert_eq!(state.hat_degrees, Some(270.0));
    }
}

#[cfg(test)]
mod rpc_tests {
    use super::*;

    #[test]
    fn unknown_request_returns_method_not_found_with_original_id() {
        for id in [json!(7), json!("request-7"), Value::Null] {
            let response =
                handle(&json!({"jsonrpc": "2.0", "id": id, "method": "unknown"})).unwrap();

            assert_eq!(response["jsonrpc"], "2.0");
            assert_eq!(response["id"], id);
            assert_eq!(response["error"]["code"], -32601);
            assert_eq!(response["error"]["message"], "Method not found");
        }
    }

    #[test]
    fn unknown_notification_is_ignored() {
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "unknown"})).is_none());
    }

    #[test]
    fn lifecycle_and_cancellation_notifications_do_not_reply() {
        for method in ["notifications/initialized", "notifications/cancelled"] {
            assert!(handle(&json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": {"requestId": 7}
            }))
            .is_none());
        }
    }

    #[test]
    fn legacy_initialize_response_is_unchanged() {
        let legacy = handle(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2025-03-26"}
        }))
        .unwrap();
        assert_eq!(legacy["result"]["protocolVersion"], "2025-03-26");
        assert!(legacy["result"].get("resultType").is_none());
        assert!(legacy["result"].get("serverInfo").is_some());
    }

    #[test]
    fn modern_discover_uses_metadata_negotiation_and_exact_result_shape() {
        let modern = handle(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {"tools": {}}
            }}
        }))
        .unwrap();
        assert_eq!(
            modern["result"],
            json!({
                "resultType": "complete",
                "supportedVersions": ["2026-07-28"],
                "capabilities": {"tools": {"listChanged": false}},
                "_meta": {"io.modelcontextprotocol/serverInfo": server_info()}
            })
        );
        assert!(modern["result"].get("protocolVersion").is_none());
        assert!(modern["result"].get("serverInfo").is_none());
    }

    #[test]
    fn modern_requests_require_capabilities_and_reject_unknown_versions() {
        let missing_capabilities = handle(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "server/discover",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28"
            }}
        }))
        .unwrap();
        assert_eq!(missing_capabilities["error"]["code"], -32602);

        let rejected = handle(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "server/discover",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2027-01-01",
                "io.modelcontextprotocol/clientCapabilities": {}
            }}
        }))
        .unwrap();
        assert_eq!(rejected["error"]["code"], -32022);
        assert_eq!(
            rejected["error"]["data"]["supported"],
            json!(["2026-07-28"])
        );
        assert_eq!(rejected["error"]["data"]["requested"], "2027-01-01");
    }

    #[test]
    fn modern_tools_results_are_complete_and_require_metadata_after_discovery() {
        let metadata = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        });
        let listed = handle(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {"_meta": metadata}
        }))
        .unwrap();
        assert_eq!(listed["result"]["resultType"], "complete");
        assert_eq!(
            listed["result"]["_meta"]["io.modelcontextprotocol/serverInfo"],
            server_info()
        );

        let tool = handle(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "shutdown",
                "arguments": {},
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }
        }))
        .unwrap();
        assert_eq!(tool["result"]["resultType"], "complete");
        assert!(tool["result"]["content"].is_array());

        let mut session = None;
        handle_with_session(
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "server/discover",
                "params": {"_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {}
                }}
            }),
            &mut session,
        );
        let missing_metadata = handle_with_session(
            &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"}),
            &mut session,
        )
        .unwrap();
        assert_eq!(missing_metadata["error"]["code"], -32602);
    }

    #[test]
    fn malformed_tool_call_is_a_protocol_error() {
        let response = handle(&json!({
            "jsonrpc": "2.0",
            "id": "bad-tool",
            "method": "tools/call",
            "params": {"name": "missing_tool", "arguments": {}}
        }))
        .unwrap();

        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn json_results_include_structured_content() {
        let result = tool_json(&vec![json!({"key": "c1"})]);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("c1"));
        assert_eq!(result["structuredContent"]["items"][0]["key"], "c1");
    }
}

#[cfg(test)]
mod validation_and_sequence_tests {
    use super::*;

    #[test]
    fn state_patch_rejects_out_of_range_analog_and_validates_sensor_fields() {
        let invalid: Patch = serde_json::from_value(json!({
            "axes": {"x": 1.01},
            "battery_level": 11,
            "accel_g": [0.0, 0.0, 1.0]
        }))
        .unwrap();
        assert!(patched_state(GamepadState::neutral(), invalid).is_err());

        let patch: Patch = serde_json::from_value(json!({
            "hat_degrees": 90.0,
            "battery_level": 8,
            "battery_charging": true,
            "accel_g": [0.0, 1.0, 0.0],
            "gyro_dps": [1.0, 2.0, 3.0]
        }))
        .unwrap();
        let state = patched_state(GamepadState::neutral(), patch).unwrap();
        assert_eq!(state.battery_level, Some(8));
        assert_eq!(state.battery_charging, Some(true));
        assert_eq!(state.accel_g, Some([0.0, 1.0, 0.0]));
        assert_eq!(state.gyro_dps, Some([1.0, 2.0, 3.0]));
    }

    #[test]
    fn sequence_is_validated_and_evolves_the_mirrored_state() {
        let args = json!({
            "actions": [
                {"at_ms": 0, "press_buttons": "a", "state": {"battery_level": 5}},
                {"at_ms": 20, "release_buttons": "a", "dpad": "east", "axes": {"x": 1.0}}
            ]
        });
        let mut actions = parse_sequence_actions(args.as_object().unwrap())
            .unwrap()
            .into_iter();
        let state =
            apply_sequence_action(GamepadState::neutral(), actions.next().unwrap()).unwrap();
        assert_eq!(state.buttons, Buttons::A);
        assert_eq!(state.battery_level, Some(5));

        let state = apply_sequence_action(state, actions.next().unwrap()).unwrap();
        assert_eq!(state.buttons, Buttons::NONE);
        assert_eq!(state.hat, Hat::East);
        assert_eq!(state.axes.get(&Axis::X), Some(&1.0));

        let unordered = json!({"actions": [
            {"at_ms": 5, "dpad": "north"},
            {"at_ms": 4, "dpad": "south"}
        ]});
        assert!(parse_sequence_actions(unordered.as_object().unwrap()).is_err());
    }

    #[test]
    fn watchdog_is_disabled_by_default_and_rejects_invalid_values() {
        assert_eq!(parse_inactivity_timeout(None), None);
        assert_eq!(parse_inactivity_timeout(Some("0")), None);
        assert_eq!(parse_inactivity_timeout(Some("bad")), None);
        assert_eq!(
            parse_inactivity_timeout(Some("25")),
            Some(Duration::from_millis(25))
        );
    }
}
