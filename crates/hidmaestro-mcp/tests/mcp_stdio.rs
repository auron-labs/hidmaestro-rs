//! Process-level MCP protocol coverage. These tests intentionally speak to the
//! compiled server over stdio rather than calling its request handler directly.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const MODERN_METADATA_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const MODERN_CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const MODERN_SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: Receiver<std::io::Result<String>>,
}

impl McpProcess {
    fn spawn(environment: &[(String, OsString)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hidmaestro-mcp"));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .envs(environment.iter().map(|(key, value)| (key, value)));
        let mut child = command.spawn().expect("spawn hidmaestro-mcp");
        let stdout = child.stdout.take().expect("piped MCP stdout");
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            stdin: Some(child.stdin.take().expect("piped MCP stdin")),
            child,
            responses,
        }
    }

    fn request(&mut self, request: Value) -> Value {
        let request_id = request.get("id").cloned();
        let stdin = self.stdin.as_mut().expect("MCP stdin is still open");
        writeln!(stdin, "{request}").expect("write MCP request");
        stdin.flush().expect("flush MCP request");

        let line = self
            .responses
            .recv_timeout(RESPONSE_TIMEOUT)
            .expect("timed out waiting for MCP response")
            .expect("read MCP response");
        let response: Value = serde_json::from_str(&line).expect("MCP response is JSON");
        assert_eq!(response.get("id"), request_id.as_ref());
        response
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn exit_within(&mut self, timeout: Duration) -> ExitStatus {
        self.close_stdin();
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Ok(None) | Err(_) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    panic!("hidmaestro-mcp did not exit within {timeout:?}");
                }
            }
        }
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        self.close_stdin();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hidmaestro-mcp-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct FakeBridge {
    executable: PathBuf,
    runs_marker: PathBuf,
    shutdown_marker: PathBuf,
    _directory: TestDirectory,
}

impl FakeBridge {
    fn new() -> Self {
        let directory = TestDirectory::new("fake-bridge");
        let script = directory.path().join("fake_bridge.py");
        std::fs::write(
            &script,
            r#"import json
import os
from pathlib import Path
import sys

runs_marker = Path(os.environ["FAKE_BRIDGE_RUNS_MARKER"])
shutdown_marker = Path(os.environ["FAKE_BRIDGE_SHUTDOWN_MARKER"])
exit_after_first_ping = not runs_marker.exists()
runs_marker.write_text("started")

for line in sys.stdin:
    request = json.loads(line)
    if request["method"] == "ping":
        print(json.dumps({"id": request["id"], "ok": True, "result": "pong"}), flush=True)
        if exit_after_first_ping:
            sys.exit(0)
    elif request["method"] == "shutdown":
        shutdown_marker.write_text("shutdown")
        print(json.dumps({"id": request["id"], "ok": True, "result": None}), flush=True)
        break
    else:
        print(json.dumps({"id": request["id"], "ok": True, "result": None}), flush=True)
"#,
        )
        .expect("write fake bridge");

        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;

            let executable = directory.path().join("fake-bridge");
            std::fs::write(
                &executable,
                format!(
                    "#!/usr/bin/env python3\n{}",
                    std::fs::read_to_string(&script).unwrap()
                ),
            )
            .expect("write fake bridge launcher");
            let mut permissions = std::fs::metadata(&executable)
                .expect("fake bridge metadata")
                .permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&executable, permissions)
                .expect("make fake bridge executable");
            executable
        };

        #[cfg(windows)]
        let executable = {
            let executable = directory.path().join("fake-bridge.cmd");
            std::fs::write(
                &executable,
                "@echo off\r\npy -3 \"%~dp0fake_bridge.py\"\r\n",
            )
            .expect("write fake bridge launcher");
            executable
        };

        Self {
            executable,
            runs_marker: directory.path().join("runs"),
            shutdown_marker: directory.path().join("shutdown"),
            _directory: directory,
        }
    }

    fn environment(&self) -> Vec<(String, OsString)> {
        vec![
            (
                "HIDMAESTRO_BRIDGE_PATH".to_string(),
                self.executable.clone().into_os_string(),
            ),
            (
                "FAKE_BRIDGE_RUNS_MARKER".to_string(),
                self.runs_marker.clone().into_os_string(),
            ),
            (
                "FAKE_BRIDGE_SHUTDOWN_MARKER".to_string(),
                self.shutdown_marker.clone().into_os_string(),
            ),
        ]
    }
}

fn modern_metadata() -> Value {
    json!({
        MODERN_METADATA_KEY: "2026-07-28",
        MODERN_CAPABILITIES_KEY: {"tools": {}}
    })
}

#[test]
fn legacy_initialize_then_tools_list_uses_the_executable_stdio_protocol() {
    let mut server = McpProcess::spawn(&[]);

    let initialized = server.request(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-03-26"}
    }));
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");

    let listed = server.request(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert!(listed["result"]["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "start")));

    assert!(server.exit_within(EXIT_TIMEOUT).success());
}

#[test]
fn modern_discovery_and_tools_list_require_metadata_on_each_request() {
    let mut server = McpProcess::spawn(&[]);

    let discovered = server.request(json!({
        "jsonrpc": "2.0",
        "id": "discover",
        "method": "server/discover",
        "params": {"_meta": modern_metadata()}
    }));
    assert_eq!(
        discovered,
        json!({
            "jsonrpc": "2.0",
            "id": "discover",
            "result": {
                "resultType": "complete",
                "supportedVersions": ["2026-07-28"],
                "capabilities": {"tools": {"listChanged": false}},
                "_meta": {MODERN_SERVER_INFO_KEY: {
                    "name": "hidmaestro-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }}
            }
        })
    );

    let listed = server.request(json!({
        "jsonrpc": "2.0",
        "id": "list",
        "method": "tools/list",
        "params": {"_meta": modern_metadata()}
    }));
    assert_eq!(listed["result"]["resultType"], "complete");
    assert!(listed["result"]["tools"].is_array());

    assert!(server.exit_within(EXIT_TIMEOUT).success());
}

#[test]
fn malformed_tool_call_is_a_protocol_error_over_stdio() {
    let mut server = McpProcess::spawn(&[]);

    let response = server.request(json!({
        "jsonrpc": "2.0",
        "id": "bad-call",
        "method": "tools/call",
        "params": {"name": "start", "arguments": []}
    }));
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(
        response["error"]["message"],
        "tools/call 'arguments' must be an object"
    );

    assert!(server.exit_within(EXIT_TIMEOUT).success());
}

#[test]
fn stdin_eof_exits_cleanly_without_starting_a_bridge() {
    let mut server = McpProcess::spawn(&[]);

    assert!(server.exit_within(EXIT_TIMEOUT).success());
}

#[test]
fn eof_shuts_down_fake_bridge_and_start_recovers_after_a_bridge_exit() {
    let fake_bridge = FakeBridge::new();
    let mut server = McpProcess::spawn(&fake_bridge.environment());

    let initialized = server.request(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-03-26"}
    }));
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");

    let first_start = server.request(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "start", "arguments": {}}
    }));
    assert_eq!(first_start["result"]["isError"], Value::Null);

    let reconnected_start = server.request(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "start", "arguments": {}}
    }));
    assert_eq!(reconnected_start["result"]["isError"], Value::Null);
    assert!(fake_bridge.runs_marker.exists());

    assert!(server.exit_within(EXIT_TIMEOUT).success());
    assert_eq!(
        std::fs::read_to_string(&fake_bridge.shutdown_marker)
            .expect("fake bridge received shutdown"),
        "shutdown"
    );
}
