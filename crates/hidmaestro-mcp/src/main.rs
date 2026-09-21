//! hidmaestro-mcp — MCP stdio server exposing HIDMaestro virtual gamepads
//! to LLM agents.
//!
//! Speaks MCP (JSON-RPC 2.0 over newline-delimited stdio). Requires the
//! `hidmaestro-bridge` executable — point `HIDMAESTRO_BRIDGE_PATH` at it or
//! put it on PATH. The bridge only functions on Windows.

use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
use std::time::Duration;

use hidmaestro::{Axis, Buttons, GamepadState, Hat, HidMaestro, StandardAxes};
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

    fn state_for(&mut self, key: &str) -> Result<&mut GamepadState, String> {
        self.hm()?
            .state_mut(key)
            .ok_or_else(|| format!("no such controller '{key}' (created by this server?)"))
    }

    fn submit(&mut self, key: &str) -> Result<(), String> {
        let state = self
            .hm()?
            .state(key)
            .cloned()
            .ok_or_else(|| format!("no such controller '{key}'"))?;
        self.hm()?
            .submit_state(key, &state)
            .map_err(|e| e.to_string())
    }
}

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
        Value::Number(n) => Ok(Buttons::from_bits_truncate(
            n.as_u64()
                .ok_or("buttons bitmask must be an unsigned integer")? as u32,
        )),
        _ => Err("buttons must be a string, array of names, or integer bitmask".into()),
    }
}

fn f(v: &Map<String, Value>, key: &str) -> Option<f32> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32)
}

#[derive(Deserialize)]
struct Patch {
    buttons: Option<Value>,
    hat: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_f32")]
    hat_degrees: Option<Option<f32>>,
    axes: Option<Map<String, Value>>,
    standard_axes: Option<StandardAxes>,
}

fn deserialize_optional_f32<'de, D>(deserializer: D) -> Result<Option<Option<f32>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Option::<f32>::deserialize(deserializer)?))
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
        state.hat_degrees = hat_degrees;
    }
    if let Some(axes) = patch.axes {
        for (k, v) in axes {
            let a = Axis::by_name(&k).ok_or_else(|| format!("unknown axis '{k}'"))?;
            let val = v.as_f64().ok_or("axis values must be numbers")? as f32;
            state.axes.insert(a, val.clamp(0.0, 1.0));
        }
    }
    if patch.standard_axes.is_some() {
        state.standard_axes = patch.standard_axes;
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
    tool_ok(serde_json::to_string_pretty(v).unwrap_or_default())
}

static APP: Mutex<Option<App>> = Mutex::new(None);

fn with_app<R>(f: impl FnOnce(&mut App) -> Result<R, String>) -> Result<R, String> {
    let mut g = APP.lock().map_err(|_| "server lock poisoned".to_string())?;
    if g.is_none() {
        *g = Some(App { hm: None });
    }
    f(g.as_mut().unwrap())
}

fn call_tool(name: &str, args: Map<String, Value>) -> Result<Value, String> {
    match name {
        // ── lifecycle ────────────────────────────────────────────────
        "start" => with_app(|app| {
            let hm = app.hm()?;
            let pong = hm.ping().map_err(|e| e.to_string())?;
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
        "create_controller" => with_app(|app| {
            let profile_id = args
                .get("profile_id")
                .and_then(|p| p.as_str())
                .ok_or("missing 'profile_id'")?;
            let index = args
                .get("index")
                .and_then(|i| i.as_u64())
                .map(|i| i as usize);
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
            app.state_for(key)?.buttons |= buttons;
            app.submit(key)?;
            if let Some(hold) = args.get("hold_ms").and_then(|h| h.as_u64()) {
                std::thread::sleep(Duration::from_millis(hold));
                app.state_for(key)?.buttons -= buttons;
                app.submit(key)?;
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
        "release_buttons" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let buttons = parse_buttons(args.get("buttons").ok_or("missing 'buttons'")?)?;
            app.state_for(key)?.buttons -= buttons;
            app.submit(key)?;
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
            app.state_for(key)?.buttons = buttons;
            app.submit(key)?;
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
            let value = f(&args, "value").ok_or("missing 'value'")?;
            let axis =
                Axis::by_name(axis_name).ok_or_else(|| format!("unknown axis '{axis_name}'"))?;
            app.state_for(key)?.axes.insert(axis, value.clamp(0.0, 1.0));
            app.submit(key)?;
            Ok(tool_ok(format!("'{key}' axis {axis_name} = {value:.3}")))
        }),
        "set_sticks" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            let st = app.state_for(key)?;
            let mut sa = st.standard_axes.unwrap_or_default();
            if let Some(v) = f(&args, "left_x") {
                sa.left_stick_x = Some(v.clamp(0.0, 1.0));
            }
            if let Some(v) = f(&args, "left_y") {
                sa.left_stick_y = Some(v.clamp(0.0, 1.0));
            }
            if let Some(v) = f(&args, "right_x") {
                sa.right_stick_x = Some(v.clamp(0.0, 1.0));
            }
            if let Some(v) = f(&args, "right_y") {
                sa.right_stick_y = Some(v.clamp(0.0, 1.0));
            }
            if let Some(v) = f(&args, "left_trigger") {
                sa.left_trigger = Some(v.clamp(0.0, 1.0));
            }
            if let Some(v) = f(&args, "right_trigger") {
                sa.right_trigger = Some(v.clamp(0.0, 1.0));
            }
            st.standard_axes = Some(sa);
            app.submit(key)?;
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
            let st = app.state_for(key)?;
            st.hat = hat;
            st.hat_degrees = None;
            app.submit(key)?;
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
            let state = app
                .hm()?
                .state(key)
                .cloned()
                .ok_or_else(|| format!("no such controller '{key}'"))?;
            let state = patched_state(state, patch)?;
            app.hm()?
                .submit_state(key, &state)
                .map_err(|e| e.to_string())?;
            Ok(tool_ok(format!("'{key}' state updated")))
        }),
        "get_state" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            Ok(tool_json(app.state_for(key)?))
        }),
        "reset_controller" => with_app(|app| {
            let key = args
                .get("controller")
                .and_then(|c| c.as_str())
                .ok_or("missing 'controller'")?;
            *app.state_for(key)? = GamepadState::neutral();
            app.submit(key)?;
            Ok(tool_ok(format!("'{key}' reset to neutral")))
        }),
        "drain_output_events" => with_app(|app| {
            let events = app.hm()?.drain_events();
            Ok(tool_json(&events))
        }),

        _ => Err(format!("unknown tool '{name}'")),
    }
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
            "inputSchema": {"type": "object", "properties": {}}
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
            }}
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
            "inputSchema": {"type": "object", "properties": {}}
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
            "description": "Hold buttons down; with hold_ms, taps instead. Names: a,b,x,y,left_bumper,right_bumper,back,start,left_stick,right_stick,guide,touchpad,share,misc1 (PlayStation aliases cross,circle,square,triangle work too).",
            "inputSchema": {"type": "object", "required": ["controller", "buttons"], "properties": {
                "controller": {"type": "string"},
                "buttons": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}, {"type": "integer"}]},
                "hold_ms": {"type": "integer", "description": "If set, press then release after this many ms"}
            }}
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
                "value": {"type": "number"}
            }}
        },
        {
            "name": "set_sticks",
            "description": "Set canonical sticks/triggers (each 0.0..1.0; omitted fields unchanged).",
            "inputSchema": {"type": "object", "required": ["controller"], "properties": {
                "controller": {"type": "string"},
                "left_x": {"type": "number"}, "left_y": {"type": "number"},
                "right_x": {"type": "number"}, "right_y": {"type": "number"},
                "left_trigger": {"type": "number"}, "right_trigger": {"type": "number"}
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
            "description": "Patch several state fields at once (buttons, hat, axes, standard_axes).",
            "inputSchema": {"type": "object", "required": ["controller", "state"], "properties": {
                "controller": {"type": "string"},
                "state": {"type": "object"}
            }}
        },
        {
            "name": "get_state",
            "description": "Return the server's mirrored state for a controller.",
            "inputSchema": {"type": "object", "required": ["controller"], "properties": {
                "controller": {"type": "string"}
            }}
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
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "shutdown",
            "description": "Stop the bridge and remove all virtual controllers.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

fn handle(msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id").cloned();

    let result: Value = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "hidmaestro-mcp", "version": env!("CARGO_PKG_VERSION")}
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .and_then(|a| a.as_object())
                .cloned()
                .unwrap_or_default();
            match call_tool(name, args) {
                Ok(v) => v,
                Err(e) => tool_error(e),
            }
        }
        _ => {
            let id = id?;
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "Method not found"}
            }));
        }
    };

    let id = id?;
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}})
                );
                let _ = stdout.flush();
                continue;
            }
        };
        if let Some(resp) = handle(&msg) {
            let _ = writeln!(stdout, "{resp}");
            let _ = stdout.flush();
        }
    }
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
}
