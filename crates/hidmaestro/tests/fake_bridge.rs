//! End-to-end test of the RPC plumbing against a fake bridge process
//! (a Python NDJSON responder), so the crate is testable off-Windows.

use std::process::Stdio;
use std::time::Duration;

use hidmaestro::{Buttons, GamepadState, Hat, HidMaestro};

const FAKE: &str = r#"
import sys, json
for line in sys.stdin:
    try:
        req = json.loads(line)
    except Exception:
        continue
    m = req.get("method"); p = req.get("params") or {}
    def resp(result=None, ok=True, error=None):
        print(json.dumps({"id": req["id"], "ok": ok, "result": result, "error": error}), flush=True)
    if m == "ping": resp("pong")
    elif m == "is_driver_installed": resp(True)
    elif m == "load_profiles": resp(3)
    elif m == "list_profiles":
        resp([{"id": "xbox-360-wired", "name": "Xbox 360", "vendor": "Microsoft"}])
    elif m == "get_profile":
        resp({"id": p["id"], "name": "Xbox 360"} if p["id"] == "xbox-360-wired" else None)
    elif m == "create_controller":
        print(json.dumps({"event": "output", "data": {"controller": "c1", "report_id": 2, "fields": {"rumble": "1"}, "raw": [2, 1], "crc_valid": True}}), flush=True)
        resp({"key": "c1", "profile_id": p["profile_id"]})
    elif m == "list_controllers": resp([{"key": "c1", "profile_id": "xbox-360-wired"}])
    elif m == "submit_state":
        st = p.get("state") or {}
        assert isinstance(st.get("buttons", 0), int) and isinstance(st.get("hat", 0), int)
        resp()
    elif m == "remove_controller": resp()
    elif m == "shutdown": resp(); sys.exit(0)
    else: resp(ok=False, error=f"unknown method {m}")
"#;

fn fake_bridge() -> String {
    let dir = std::env::temp_dir().join(format!("hm-fake-bridge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("fake_bridge.py");
    std::fs::write(&script, FAKE).unwrap();
    script.to_string_lossy().into_owned()
}

#[test]
fn rpc_roundtrip_against_fake_bridge() {
    let script = fake_bridge();
    let python = ["python3", "python"]
        .iter()
        .copied()
        .find(|p| {
            std::process::Command::new(p)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .expect("python3 required for the fake bridge test");

    let mut hm = HidMaestro::builder()
        .bridge_path(python)
        .arg(script)
        .response_timeout(Duration::from_secs(10))
        .spawn()
        .expect("spawn fake bridge");

    assert_eq!(hm.ping().unwrap(), "pong");
    assert!(hm.is_driver_installed().unwrap());
    assert_eq!(hm.load_default_profiles().unwrap(), 3);
    assert_eq!(hm.list_profiles().unwrap()[0].id, "xbox-360-wired");
    assert!(hm.get_profile("xbox-360-wired").unwrap().is_some());
    assert!(hm.get_profile("nope").unwrap().is_none());

    let key = hm.create_controller("xbox-360-wired", None).unwrap();
    assert_eq!(key, "c1");

    // create_controller emitted an output event before its response.
    let events = hm.drain_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].controller, "c1");
    assert_eq!(events[0].fields["rumble"], "1");

    let mut pad = hm.controller(&key).unwrap();
    pad.press(Buttons::A | Buttons::START).unwrap();
    pad.set_hat(Hat::East).unwrap();
    pad.release(Buttons::A).unwrap();
    drop(pad);

    let mut state = GamepadState::neutral();
    state.buttons = Buttons::X;
    hm.submit_state("c1", &state).unwrap();

    let controllers = hm.list_controllers().unwrap();
    assert_eq!(controllers[0].key, "c1");

    hm.remove_controller("c1").unwrap();
    hm.shutdown();
}
