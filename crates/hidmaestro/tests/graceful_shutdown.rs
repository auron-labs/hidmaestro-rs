use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use hidmaestro::HidMaestro;

const BRIDGE: &str = r#"
import json, pathlib, sys, time
marker = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    request = json.loads(line)
    if request["method"] == "shutdown":
        print(json.dumps({"id": request["id"], "ok": True, "result": None}), flush=True)
        time.sleep(0.05)
        marker.write_text("cleanup")
        break
"#;

fn python() -> &'static str {
    ["python3", "python"]
        .into_iter()
        .find(|python| {
            Command::new(python)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .expect("python3 required for bridge tests")
}

fn test_dir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hm-graceful-shutdown-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

fn bridge(marker: &std::path::Path) -> HidMaestro {
    let script = marker.with_extension("py");
    std::fs::write(&script, BRIDGE).unwrap();

    HidMaestro::builder()
        .bridge_path(python())
        .arg(script.to_string_lossy())
        .arg(marker.to_string_lossy())
        .spawn()
        .unwrap()
}

#[test]
fn shutdown_waits_for_cleanup() {
    let marker = test_dir().join("shutdown");
    bridge(&marker).shutdown();

    assert_eq!(std::fs::read_to_string(marker).unwrap(), "cleanup");
}

#[test]
fn drop_waits_for_cleanup() {
    let marker = test_dir().join("shutdown");
    {
        let _bridge = bridge(&marker);
    }

    assert_eq!(std::fs::read_to_string(marker).unwrap(), "cleanup");
}
