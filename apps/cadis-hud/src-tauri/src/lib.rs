use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde_json::Value;
use tauri::Manager;

const CADIS_CONFIG_RELATIVE_PATH: &str = ".cadis/config.toml";
const CADIS_SOCKET_RELATIVE_PATH: &str = ".cadis/run/cadisd.sock";

#[derive(Default)]
struct TtsPlaybackState {
    active_pid: Mutex<Option<u32>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalSttResult {
    text: String,
    latency_ms: u128,
}

#[tauri::command(rename_all = "camelCase")]
async fn cadis_request(request: Value, socket_path: Option<String>) -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let socket_path = discover_socket_path(socket_path)?;
        send_cadis_request(&socket_path, request).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("CADIS request worker failed: {error}"))?
}

#[tauri::command]
fn window_start_dragging(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "camelCase")]
async fn edge_tts_speak(
    state: tauri::State<'_, Arc<TtsPlaybackState>>,
    text: String,
    voice_id: String,
    rate: String,
    pitch: String,
    volume: String,
) -> Result<(), String> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        edge_tts_speak_blocking(&state, text, voice_id, rate, pitch, volume)
    })
    .await
    .map_err(|error| format!("TTS worker failed: {error}"))?
}

#[tauri::command]
fn edge_tts_stop(state: tauri::State<'_, Arc<TtsPlaybackState>>) -> Result<(), String> {
    stop_active_tts(state.inner())
}

#[tauri::command(rename_all = "camelCase")]
async fn local_stt_transcribe(audio_base64: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || local_stt_transcribe_blocking(audio_base64))
        .await
        .map_err(|error| format!("STT worker failed: {error}"))?
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
        .manage(Arc::new(TtsPlaybackState::default()))
        .invoke_handler(tauri::generate_handler![
            cadis_request,
            window_start_dragging,
            edge_tts_speak,
            edge_tts_stop,
            local_stt_transcribe,
            voice_tts_speak,
            voice_tts_stop,
            voice_stt_start,
            voice_stt_stop
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = set_cadis_window_icon(&window);
                install_microphone_permission_handler(&window);
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.center();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run CADIS HUD");
}

fn set_cadis_window_icon(window: &tauri::WebviewWindow) -> Result<(), String> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))
        .map_err(|error| format!("could not load CADIS icon: {error}"))?;
    window
        .set_icon(icon)
        .map_err(|error| format!("could not set CADIS window icon: {error}"))
}

#[cfg(target_os = "linux")]
fn install_microphone_permission_handler(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| {
        use webkit2gtk::glib::prelude::Cast;
        use webkit2gtk::{
            PermissionRequestExt, UserMediaPermissionRequest, UserMediaPermissionRequestExt,
            WebViewExt,
        };

        webview.inner().connect_permission_request(|_, request| {
            let Some(user_media) = request.dynamic_cast_ref::<UserMediaPermissionRequest>() else {
                return false;
            };

            if user_media.is_for_audio_device() && !user_media.is_for_video_device() {
                user_media.allow();
                return true;
            }

            false
        });
    });
}

#[cfg(not(target_os = "linux"))]
fn install_microphone_permission_handler(_window: &tauri::WebviewWindow) {}

fn edge_tts_speak_blocking(
    state: &Arc<TtsPlaybackState>,
    text: String,
    voice_id: String,
    rate: String,
    pitch: String,
    volume: String,
) -> Result<(), String> {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("empty TTS text".to_owned());
    }
    if text.chars().count() > 8_000 {
        return Err("TTS text is too long".to_owned());
    }

    stop_active_tts(state)?;

    let path = temp_audio_path("cadis-edge-tts", "mp3")?;
    let synth_result = synthesize_edge_tts(&path, &text, &voice_id, &rate, &pitch, &volume);
    if let Err(error) = synth_result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }

    let playback_result = play_audio_file(state, &path);
    let _ = fs::remove_file(&path);
    playback_result
}

fn local_stt_transcribe_blocking(audio_base64: String) -> Result<Value, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_base64.as_bytes())
        .map_err(|error| format!("invalid STT audio payload: {error}"))?;
    if bytes.is_empty() {
        return Err("empty STT audio".to_owned());
    }
    if bytes.len() > 25 * 1024 * 1024 {
        return Err("STT audio is too large".to_owned());
    }

    let path = write_temp_bytes("cadis-stt", "wav", &bytes)?;
    let started = Instant::now();
    let result = run_whisper_cli(&path);
    let _ = fs::remove_file(&path);
    result.map(|text| {
        serde_json::json!(LocalSttResult {
            text,
            latency_ms: started.elapsed().as_millis(),
        })
    })
}

fn write_temp_bytes(prefix: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = temp_audio_path(prefix, ext)?;
    fs::write(&path, bytes).map_err(|error| format!("cannot write temporary audio: {error}"))?;
    Ok(path)
}

fn run_whisper_cli(path: &Path) -> Result<String, String> {
    let model = whisper_model_path()?;
    let library_path = whisper_library_path_env();
    let mut last_error = String::new();

    for binary in whisper_cli_candidates() {
        let mut command = Command::new(&binary);
        command
            .arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(path)
            .arg("-nt")
            .arg("-np")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(library_path) = &library_path {
            command.env("LD_LIBRARY_PATH", library_path);
        }

        let output = match command.output() {
            Ok(output) => output,
            Err(error) => {
                last_error = format!("{}: {error}", binary.display());
                continue;
            }
        };

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        last_error = if stderr.is_empty() {
            format!("{} exited with status {}", binary.display(), output.status)
        } else {
            format!("{}: {stderr}", binary.display())
        };
    }

    Err(format!(
        "whisper-cli not available ({})",
        explain_whisper_error(&last_error)
    ))
}

fn whisper_model_path() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    push_env_path(&mut candidates, "CADIS_WHISPER_MODEL");
    push_env_path(&mut candidates, "WHISPER_MODEL");
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".local/share/cadis/whisper-models/ggml-base.en.bin"));
        candidates.push(home.join(".local/share/ramaclaw/whisper-models/ggml-base.en.bin"));
    }

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    let searched = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "whisper model not found; set CADIS_WHISPER_MODEL or install ggml-base.en.bin under ~/.local/share/cadis/whisper-models ({searched})"
    ))
}

fn whisper_cli_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    push_env_path(&mut candidates, "CADIS_WHISPER_CLI");
    push_env_path(&mut candidates, "WHISPER_CLI");
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".local/bin/whisper-cli"));
    }
    candidates.push(PathBuf::from("whisper-cli"));
    candidates
}

fn whisper_library_path_env() -> Option<String> {
    let mut paths = Vec::new();
    if let Ok(existing) = env::var("LD_LIBRARY_PATH") {
        if !existing.trim().is_empty() {
            paths.push(existing);
        }
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        for path in [
            home.join(".local/lib"),
            home.join(".local/lib64"),
            home.join(".local/share/cadis/lib"),
            home.join(".local/share/ramaclaw/lib"),
            home.join(".local/share/whisper.cpp"),
        ] {
            if path.exists() {
                paths.push(path.display().to_string());
            }
        }
    }

    if paths.is_empty() {
        None
    } else {
        Some(paths.join(":"))
    }
}

fn explain_whisper_error(error: &str) -> String {
    if error.contains("libwhisper.so") {
        format!(
            "{error}; libwhisper.so.1 is missing from the dynamic linker path. Reinstall whisper.cpp or launch CADIS HUD with LD_LIBRARY_PATH pointing to the directory that contains libwhisper.so.1"
        )
    } else {
        error.to_owned()
    }
}

fn synthesize_edge_tts(
    out_path: &Path,
    text: &str,
    voice_id: &str,
    rate: &str,
    pitch: &str,
    volume: &str,
) -> Result<(), String> {
    let input = serde_json::json!({
        "outPath": out_path,
        "text": text,
        "voiceId": voice_id,
        "rate": rate,
        "pitch": pitch,
        "volume": volume,
    })
    .to_string();

    let script = r#"
const chunks = [];
for await (const chunk of process.stdin) chunks.push(Buffer.from(chunk));
const input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
const fs = await import('node:fs/promises');
const { EdgeTTS } = await import('edge-tts-universal');
const tts = new EdgeTTS(input.text, input.voiceId, {
  rate: input.rate,
  pitch: input.pitch,
  volume: input.volume,
});
const result = await tts.synthesize();
const audio = Buffer.from(await result.audio.arrayBuffer());
await fs.writeFile(input.outPath, audio);
"#;

    let project_root = project_root()?;
    let mut last_error = String::new();
    for node in node_candidates() {
        let mut child = match Command::new(&node)
            .arg("--input-type=module")
            .arg("-e")
            .arg(script)
            .current_dir(&project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                last_error = format!("{}: {error}", node.display());
                continue;
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .map_err(|error| format!("failed to send TTS request to node: {error}"))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|error| format!("edge tts process failed: {error}"))?;
        if output.status.success() {
            return Ok(());
        }

        last_error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if last_error.is_empty() {
            last_error = format!("node exited with status {}", output.status);
        }
    }

    Err(format!("edge tts failed ({last_error})"))
}

fn play_audio_file(state: &Arc<TtsPlaybackState>, path: &Path) -> Result<(), String> {
    let player = audio_player_command(path)?;
    let mut child = Command::new(&player.program)
        .args(&player.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start audio player '{}': {error}", player.program))?;

    let pid = child.id();
    {
        let mut active_pid = state
            .active_pid
            .lock()
            .map_err(|_| "TTS state lock was poisoned".to_owned())?;
        *active_pid = Some(pid);
    }

    let status = child
        .wait()
        .map_err(|error| format!("audio player failed: {error}"))?;
    let was_cancelled = {
        let mut active_pid = state
            .active_pid
            .lock()
            .map_err(|_| "TTS state lock was poisoned".to_owned())?;
        if *active_pid == Some(pid) {
            *active_pid = None;
            false
        } else {
            true
        }
    };

    if status.success() || was_cancelled {
        Ok(())
    } else {
        Err(format!("audio player exited with status {status}"))
    }
}

fn stop_active_tts(state: &Arc<TtsPlaybackState>) -> Result<(), String> {
    let pid = {
        let mut active_pid = state
            .active_pid
            .lock()
            .map_err(|_| "TTS state lock was poisoned".to_owned())?;
        active_pid.take()
    };

    if let Some(pid) = pid {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
    }
    Ok(())
}

struct AudioPlayerCommand {
    program: String,
    args: Vec<String>,
}

fn audio_player_command(path: &Path) -> Result<AudioPlayerCommand, String> {
    let path = path.display().to_string();
    if command_exists("ffplay") {
        return Ok(AudioPlayerCommand {
            program: "ffplay".to_owned(),
            args: vec![
                "-nodisp".to_owned(),
                "-autoexit".to_owned(),
                "-loglevel".to_owned(),
                "error".to_owned(),
                path,
            ],
        });
    }
    if command_exists("mpv") {
        return Ok(AudioPlayerCommand {
            program: "mpv".to_owned(),
            args: vec![
                "--no-terminal".to_owned(),
                "--really-quiet".to_owned(),
                path,
            ],
        });
    }
    Err("no supported audio player found; install ffmpeg/ffplay or mpv".to_owned())
}

fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn project_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve CADIS HUD project root".to_owned())
}

fn node_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    push_env_path(&mut candidates, "CADIS_HUD_NODE");
    push_env_path(&mut candidates, "NODE");
    if let Ok(nvm_bin) = env::var("NVM_BIN") {
        if !nvm_bin.trim().is_empty() {
            candidates.push(PathBuf::from(nvm_bin).join("node"));
        }
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".nvm/versions/node/v24.15.0/bin/node"));
    }
    candidates.push(PathBuf::from("node"));
    candidates
}

fn push_env_path(candidates: &mut Vec<PathBuf>, key: &str) {
    if let Ok(value) = env::var(key) {
        let value = value.trim();
        if !value.is_empty() {
            candidates.push(PathBuf::from(value));
        }
    }
}

fn temp_audio_path(prefix: &str, ext: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    Ok(env::temp_dir().join(format!("{prefix}-{}-{stamp}.{ext}", std::process::id())))
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
