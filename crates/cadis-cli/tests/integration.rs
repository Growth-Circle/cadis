use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

/// Resolve the `cadisd` binary path from the same target directory as the test binary.
fn cadisd_bin() -> PathBuf {
    // Try CARGO_BIN_EXE first (set by cargo when running workspace tests).
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_cadisd") {
        let path = PathBuf::from(p);
        if path.exists() {
            return path;
        }
    }
    let mut path = std::env::current_exe().expect("current_exe");
    // Walk up from the test binary to the target profile directory.
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("cadisd");
    if !path.exists() {
        // Binary may not be built yet in some CI configurations.
        // Return the path anyway; spawn_daemon_stdio will fail with a clear error.
    }
    path
}

/// Skip-guard: returns true if cadisd binary is available.
fn cadisd_available() -> bool {
    cadisd_bin().exists()
}

/// Spawn `cadisd --stdio` with an isolated CADIS_HOME.
fn spawn_daemon_stdio() -> (std::process::Child, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let child = Command::new(cadisd_bin())
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("CADIS_HOME", tmp.path())
        .spawn()
        .expect("cadisd should start");
    (child, tmp)
}

/// Send a JSON request line and collect all response frames.
fn send_and_collect(child: &mut std::process::Child, request: &Value) -> Vec<Value> {
    let stdin = child.stdin.as_mut().expect("stdin");
    serde_json::to_writer(&mut *stdin, request).expect("write request");
    stdin.write_all(b"\n").expect("write newline");
    // Close stdin so the daemon finishes processing and exits.
    drop(child.stdin.take());

    let stdout = child.stdout.take().expect("stdout");
    BufReader::new(stdout)
        .lines()
        .filter_map(|line| {
            let line = line.expect("read line");
            if line.trim().is_empty() {
                return None;
            }
            Some(serde_json::from_str::<Value>(&line).expect("parse frame"))
        })
        .collect()
}

/// Build a minimal request envelope.
fn request(req_type: &str, payload: Value) -> Value {
    serde_json::json!({
        "protocol_version": "0.1",
        "request_id": "req_test_1",
        "client_id": "cli_test",
        "type": req_type,
        "payload": payload,
    })
}

#[test]
fn cli_status_returns_daemon_info() {
    if !cadisd_available() {
        eprintln!("skipping: cadisd not built");
        return;
    }
    let (mut child, _tmp) = spawn_daemon_stdio();
    let req = request("daemon.status", serde_json::json!({}));
    let frames = send_and_collect(&mut child, &req);

    assert!(!frames.is_empty(), "expected at least one frame");
    let resp = &frames[0];
    assert_eq!(resp["frame"], "response");
    let payload = &resp["payload"];
    assert_eq!(payload["type"], "daemon.status.response");
    let inner = &payload["payload"];
    assert!(inner["status"].is_string(), "status should be a string");
    assert!(inner["version"].is_string(), "version should be a string");
    assert!(
        inner["model_provider"].is_string(),
        "model_provider should be present"
    );
}

#[test]
fn cli_chat_returns_response() {
    if !cadisd_available() {
        eprintln!("skipping: cadisd not built");
        return;
    }
    let (mut child, _tmp) = spawn_daemon_stdio();
    let req = request(
        "message.send",
        serde_json::json!({
            "content": "hello",
            "content_kind": "chat",
        }),
    );
    let frames = send_and_collect(&mut child, &req);

    assert!(!frames.is_empty(), "expected at least one frame");
    let has_completed = frames.iter().any(|f| {
        f.get("frame")
            .and_then(|v| v.as_str())
            .map(|s| s == "event")
            .unwrap_or(false)
            && f.get("payload")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
                .map(|s| s == "message.completed")
                .unwrap_or(false)
    });
    assert!(has_completed, "expected a message.completed event");
}

#[test]
fn cli_doctor_runs_without_error() {
    if !cadisd_available() {
        eprintln!("skipping: cadisd not built");
        return;
    }
    // Doctor connects via socket; test the underlying daemon status request via stdio instead.
    let (mut child, _tmp) = spawn_daemon_stdio();
    let req = request("daemon.status", serde_json::json!({}));
    let frames = send_and_collect(&mut child, &req);

    assert!(!frames.is_empty());
    let resp = &frames[0];
    assert_eq!(resp["frame"], "response");
    let payload_type = resp["payload"]["type"].as_str().unwrap_or("");
    assert_ne!(
        payload_type, "request.rejected",
        "doctor status check should not be rejected"
    );
}

#[test]
fn cli_models_lists_providers() {
    if !cadisd_available() {
        eprintln!("skipping: cadisd not built");
        return;
    }
    let (mut child, _tmp) = spawn_daemon_stdio();
    let req = request("models.list", serde_json::json!({}));
    let frames = send_and_collect(&mut child, &req);

    assert!(!frames.is_empty(), "expected at least one frame");

    let models_event = frames
        .iter()
        .find(|f| {
            f.get("payload")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
                == Some("models.list.response")
        })
        .expect("expected a models.list.response event");
    let models = &models_event["payload"]["payload"]["models"];
    assert!(models.is_array(), "models should be an array");
    assert!(!models.as_array().unwrap().is_empty(), "models not empty");

    for model in models.as_array().unwrap() {
        assert!(
            model["provider"].is_string(),
            "each model should have a provider"
        );
    }
}

#[test]
fn cli_agents_lists_default_agents() {
    if !cadisd_available() {
        eprintln!("skipping: cadisd not built");
        return;
    }
    let (mut child, _tmp) = spawn_daemon_stdio();
    let req = request("agent.list", serde_json::json!({}));
    let frames = send_and_collect(&mut child, &req);

    assert!(!frames.is_empty(), "expected at least one frame");

    let agents_event = frames
        .iter()
        .find(|f| {
            f.get("payload")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
                == Some("agent.list.response")
        })
        .expect("expected an agent.list.response event");
    let agents = &agents_event["payload"]["payload"]["agents"];
    assert!(agents.is_array(), "agents should be an array");
    assert!(!agents.as_array().unwrap().is_empty(), "agents not empty");

    let has_main = agents.as_array().unwrap().iter().any(|a| {
        a.get("role")
            .and_then(|v| v.as_str())
            .map(|s| s == "main" || s == "Main")
            .unwrap_or(false)
    });
    assert!(has_main, "expected a main agent in the roster");
}
