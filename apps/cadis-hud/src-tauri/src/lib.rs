use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::Value;

const CADIS_CONFIG_RELATIVE_PATH: &str = ".cadis/config.toml";
const CADIS_SOCKET_RELATIVE_PATH: &str = ".cadis/run/cadisd.sock";

#[tauri::command(rename_all = "camelCase")]
async fn cadis_request(request: Value, socket_path: Option<String>) -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let socket_path = discover_socket_path(socket_path)?;
        send_cadis_request(&socket_path, request).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("CADIS request worker failed: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
fn edge_tts_speak(
    text: String,
    voice_id: String,
    rate: String,
    pitch: String,
    volume: String,
) -> Result<(), String> {
    let _ = (voice_id, rate, pitch, volume);
    if text.trim().is_empty() {
        return Err("empty TTS text".to_owned());
    }
    Ok(())
}

#[tauri::command]
fn edge_tts_stop() -> Result<(), String> {
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
fn local_stt_transcribe(audio_base64: String) -> Result<Value, String> {
    let _ = audio_base64;
    Err("local STT is not configured in this CADIS HUD build".to_owned())
}

#[tauri::command]
fn voice_tts_speak(_text: String, _voice_id: Option<String>) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
fn voice_tts_stop() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
fn voice_stt_start() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
fn voice_stt_stop() -> Result<(), String> {
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cadis_request,
            edge_tts_speak,
            edge_tts_stop,
            local_stt_transcribe,
            voice_tts_speak,
            voice_tts_stop,
            voice_stt_start,
            voice_stt_stop
        ])
        .run(tauri::generate_context!())
        .expect("failed to run CADIS HUD");
}

fn send_cadis_request(socket_path: &Path, request: Value) -> io::Result<Vec<Value>> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not connect to cadisd at {}: {error}",
                socket_path.display()
            ),
        )
    })?;

    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;

    read_json_lines(stream)
}

fn read_json_lines(stream: UnixStream) -> io::Result<Vec<Value>> {
    let reader = BufReader::new(stream);
    let mut frames = Vec::new();

    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value = serde_json::from_str::<Value>(line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "cadisd returned invalid JSON on line {}: {error}",
                    index + 1
                ),
            )
        })?;
        frames.push(value);
    }

    Ok(frames)
}

fn discover_socket_path(explicit: Option<String>) -> Result<PathBuf, String> {
    let env = DiscoveryEnv::from_process();
    discover_socket_path_with_env(explicit, &env)
}

fn discover_socket_path_with_env(
    explicit: Option<String>,
    env: &DiscoveryEnv,
) -> Result<PathBuf, String> {
    if let Some(path) = non_empty(explicit) {
        return expand_home(&path, env).map_err(|error| error.to_string());
    }

    if let Some(path) = non_empty(env.cadis_hud_socket.clone()) {
        return expand_home(&path, env).map_err(|error| error.to_string());
    }

    if let Some(path) = non_empty(env.cadis_socket.clone()) {
        return expand_home(&path, env).map_err(|error| error.to_string());
    }

    if let Some(path) = config_socket_path(env)? {
        return expand_home(&path, env).map_err(|error| error.to_string());
    }

    if let Some(runtime_dir) = non_empty(env.xdg_runtime_dir.clone()) {
        return Ok(PathBuf::from(runtime_dir).join("cadis").join("cadisd.sock"));
    }

    let home = env
        .home
        .as_ref()
        .ok_or_else(|| "could not resolve CADIS socket path because HOME is unset".to_owned())?;
    Ok(home.join(CADIS_SOCKET_RELATIVE_PATH))
}

fn config_socket_path(env: &DiscoveryEnv) -> Result<Option<String>, String> {
    let Some(home) = env.home.as_ref() else {
        return Ok(None);
    };
    let config_path = home.join(CADIS_CONFIG_RELATIVE_PATH);
    let contents = match fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read CADIS config at {}: {error}",
                config_path.display()
            ));
        }
    };

    let value = contents.parse::<toml::Value>().map_err(|error| {
        format!(
            "could not parse CADIS config at {}: {error}",
            config_path.display()
        )
    })?;

    Ok(value
        .get("socket_path")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .and_then(|value| non_empty(Some(value))))
}

fn expand_home(path: &str, env: &DiscoveryEnv) -> io::Result<PathBuf> {
    if path == "~" {
        return env
            .home
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unset"));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        let home = env
            .home
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unset"))?;
        return Ok(home.join(rest));
    }

    Ok(PathBuf::from(path))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

#[derive(Debug, Default)]
struct DiscoveryEnv {
    cadis_hud_socket: Option<String>,
    cadis_socket: Option<String>,
    home: Option<PathBuf>,
    xdg_runtime_dir: Option<String>,
}

impl DiscoveryEnv {
    fn from_process() -> Self {
        Self {
            cadis_hud_socket: env::var("CADIS_HUD_SOCKET").ok(),
            cadis_socket: env::var("CADIS_SOCKET").ok(),
            home: env::var_os("HOME").map(PathBuf::from),
            xdg_runtime_dir: env::var("XDG_RUNTIME_DIR").ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn discovery_prefers_explicit_socket_path() {
        let env = DiscoveryEnv {
            cadis_hud_socket: Some("/tmp/hud.sock".to_owned()),
            cadis_socket: Some("/tmp/cadis.sock".to_owned()),
            home: Some(PathBuf::from("/home/cadis")),
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
        };

        let socket_path =
            discover_socket_path_with_env(Some("~/explicit.sock".to_owned()), &env).unwrap();

        assert_eq!(socket_path, PathBuf::from("/home/cadis/explicit.sock"));
    }

    #[test]
    fn discovery_prefers_hud_env_over_generic_env() {
        let env = DiscoveryEnv {
            cadis_hud_socket: Some("/tmp/hud.sock".to_owned()),
            cadis_socket: Some("/tmp/cadis.sock".to_owned()),
            home: Some(PathBuf::from("/home/cadis")),
            xdg_runtime_dir: None,
        };

        let socket_path = discover_socket_path_with_env(None, &env).unwrap();

        assert_eq!(socket_path, PathBuf::from("/tmp/hud.sock"));
    }

    #[test]
    fn discovery_uses_config_before_runtime_default() {
        let home = unique_temp_dir();
        fs::create_dir_all(home.join(".cadis")).unwrap();
        fs::write(
            home.join(CADIS_CONFIG_RELATIVE_PATH),
            "socket_path = \"~/.cadis/custom.sock\"\n",
        )
        .unwrap();
        let env = DiscoveryEnv {
            home: Some(home.clone()),
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
            ..DiscoveryEnv::default()
        };

        let socket_path = discover_socket_path_with_env(None, &env).unwrap();

        assert_eq!(socket_path, home.join(".cadis/custom.sock"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn discovery_uses_xdg_runtime_dir_before_home_default() {
        let env = DiscoveryEnv {
            home: Some(PathBuf::from("/home/cadis")),
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
            ..DiscoveryEnv::default()
        };

        let socket_path = discover_socket_path_with_env(None, &env).unwrap();

        assert_eq!(
            socket_path,
            PathBuf::from("/run/user/1000/cadis/cadisd.sock")
        );
    }

    #[test]
    fn cadis_request_writes_one_json_line_and_reads_frames() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let socket_path = dir.join("cadisd.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(line.trim(), r#"{"type":"daemon.status"}"#);
            stream.write_all(b"{\"type\":\"request.accepted\"}\n\n{\"type\":\"daemon.status.response\",\"payload\":{\"status\":\"ok\"}}\n").unwrap();
        });

        let frames = send_cadis_request(
            &socket_path,
            serde_json::json!({
                "type": "daemon.status"
            }),
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(
            frames,
            vec![
                serde_json::json!({"type": "request.accepted"}),
                serde_json::json!({"type": "daemon.status.response", "payload": {"status": "ok"}})
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("cadis-hud-test-{}-{nanos}", std::process::id()))
    }
}
