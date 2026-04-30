use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde_json::Value;
use tauri::{Emitter, Manager};

const CADIS_CONFIG_RELATIVE_PATH: &str = ".cadis/config.toml";
#[cfg(unix)]
const CADIS_SOCKET_RELATIVE_PATH: &str = ".cadis/run/cadisd.sock";
const DEFAULT_TCP_ADDRESS: &str = "127.0.0.1:7433";
const CADIS_FRAME_EVENT: &str = "cadis-frame";
const CADIS_SUBSCRIPTION_CLOSED_EVENT: &str = "cadis-subscription-closed";

/// Transport-agnostic stream that wraps either a Unix socket or a TCP connection.
enum DaemonStream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for DaemonStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for DaemonStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

impl DaemonStream {
    fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.shutdown(how),
            Self::Tcp(s) => s.shutdown(how),
        }
    }

    fn try_clone(&self) -> io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.try_clone().map(Self::Unix),
            Self::Tcp(s) => s.try_clone().map(Self::Tcp),
        }
    }
}

#[derive(Default)]
struct TtsPlaybackState {
    active_pid: Mutex<Option<u32>>,
}

#[derive(Default)]
struct CadisSubscriptionState {
    generation: AtomicU64,
    stream: Mutex<Option<DaemonStream>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalSttResult {
    text: String,
    latency_ms: u128,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceDoctorCheck {
    name: String,
    status: String,
    detail: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceDoctorReport {
    summary: String,
    checks: Vec<VoiceDoctorCheck>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CadisSubscriptionClosed {
    generation: u64,
    error: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
async fn cadis_request(request: Value, socket_path: Option<String>) -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let transport = discover_transport(socket_path)?;
        send_cadis_request(&transport, request).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("CADIS request worker failed: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn cadis_events_subscribe(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<CadisSubscriptionState>>,
    request: Value,
    socket_path: Option<String>,
) -> Result<(), String> {
    let state = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let transport = discover_transport(socket_path)?;
        start_cadis_event_subscription(app, state, &transport, request)
    })
    .await
    .map_err(|error| format!("CADIS subscription worker failed: {error}"))?
}

#[tauri::command]
fn cadis_events_unsubscribe(
    state: tauri::State<'_, Arc<CadisSubscriptionState>>,
) -> Result<(), String> {
    state.inner().close_active_subscription()
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
async fn local_stt_transcribe(
    audio_base64: String,
    language: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        local_stt_transcribe_blocking(audio_base64, language)
    })
    .await
    .map_err(|error| format!("STT worker failed: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn voice_doctor_preflight(
    renderer_mic: VoiceDoctorCheck,
) -> Result<VoiceDoctorReport, String> {
    tauri::async_runtime::spawn_blocking(move || voice_doctor_preflight_blocking(renderer_mic))
        .await
        .map_err(|error| format!("voice doctor worker failed: {error}"))
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

#[tauri::command]
fn open_in_editor(path: String) -> Result<(), String> {
    // Desktop convenience: open a CADIS-owned worktree in the user's editor.
    // Validate the canonical path contains a .cadis/worktrees/ segment.
    let canonical = std::fs::canonicalize(&path)
        .map_err(|e| format!("cannot resolve path: {e}"))?;
    let canonical_str = canonical.to_string_lossy();
    if !canonical_str.contains("/.cadis/worktrees/") && !canonical_str.contains("\\.cadis\\worktrees\\") {
        return Err("path is not inside a CADIS-owned worktree".to_owned());
    }
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "code".to_owned());
    std::process::Command::new(&editor)
        .arg(&canonical)
        .spawn()
        .map_err(|e| format!("failed to open editor: {e}"))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(TtsPlaybackState::default()))
        .manage(Arc::new(CadisSubscriptionState::default()))
        .invoke_handler(tauri::generate_handler![
            cadis_request,
            cadis_events_subscribe,
            cadis_events_unsubscribe,
            window_start_dragging,
            edge_tts_speak,
            edge_tts_stop,
            local_stt_transcribe,
            voice_doctor_preflight,
            voice_tts_speak,
            voice_tts_stop,
            voice_stt_start,
            voice_stt_stop,
            open_in_editor
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
            PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
            UserMediaPermissionRequestExt, WebViewExt,
        };

        let inner = webview.inner();
        if let Some(settings) = inner.settings() {
            settings.set_enable_media_stream(true);
        }

        inner.connect_permission_request(|_, request| {
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

fn voice_doctor_preflight_blocking(renderer_mic: VoiceDoctorCheck) -> VoiceDoctorReport {
    let mut checks = vec![renderer_mic];
    checks.push(whisper_binary_check());
    checks.push(whisper_model_check());
    checks.push(node_helper_check());
    checks.push(audio_player_check());

    let failures = checks.iter().filter(|check| check.status == "fail").count();
    let warnings = checks.iter().filter(|check| check.status == "warn").count();
    let summary = if failures > 0 {
        format!("{failures} blocking issue{}", plural(failures))
    } else if warnings > 0 {
        format!("{warnings} warning{}", plural(warnings))
    } else {
        "ready".to_owned()
    };

    VoiceDoctorReport { summary, checks }
}

fn whisper_binary_check() -> VoiceDoctorCheck {
    for candidate in whisper_cli_candidates() {
        if let Some(path) = resolve_candidate_path(&candidate) {
            return doctor_check(
                "whisper binary",
                "pass",
                format!("found {}", path.display()),
            );
        }
    }

    let searched = whisper_cli_candidates()
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    doctor_check("whisper binary", "fail", format!("not found ({searched})"))
}

fn whisper_model_check() -> VoiceDoctorCheck {
    match whisper_model_path() {
        Ok(path) => doctor_check("whisper model", "pass", format!("found {}", path.display())),
        Err(error) => doctor_check("whisper model", "fail", error),
    }
}

fn node_helper_check() -> VoiceDoctorCheck {
    let project_root = match project_root() {
        Ok(path) => path,
        Err(error) => return doctor_check("node helper", "fail", error),
    };

    let script = "await import('edge-tts-universal')";
    let mut found_node = None;
    let mut last_error = String::new();
    for node in node_candidates() {
        let Some(path) = resolve_candidate_path(&node) else {
            continue;
        };
        found_node = Some(path.clone());
        match Command::new(&path)
            .arg("--input-type=module")
            .arg("-e")
            .arg(script)
            .current_dir(&project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(output) if output.status.success() => {
                return doctor_check("node helper", "pass", format!("node {}", path.display()));
            }
            Ok(output) => {
                last_error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                if last_error.is_empty() {
                    last_error = format!("node exited with status {}", output.status);
                }
            }
            Err(error) => {
                last_error = error.to_string();
            }
        }
    }

    if let Some(path) = found_node {
        doctor_check(
            "node helper",
            "fail",
            format!(
                "{} cannot load edge-tts-universal ({})",
                path.display(),
                concise_error(&last_error)
            ),
        )
    } else {
        doctor_check("node helper", "fail", "node not found".to_owned())
    }
}

fn audio_player_check() -> VoiceDoctorCheck {
    let mut players = Vec::new();
    for program in ["ffplay", "mpv"] {
        if let Some(path) = resolve_program_path(program) {
            players.push(format!("{program}: {}", path.display()));
        }
    }

    if players.is_empty() {
        doctor_check(
            "audio player",
            "fail",
            "install ffmpeg/ffplay or mpv".to_owned(),
        )
    } else {
        doctor_check("audio player", "pass", players.join("; "))
    }
}

fn doctor_check(name: &str, status: &str, detail: String) -> VoiceDoctorCheck {
    VoiceDoctorCheck {
        name: name.to_owned(),
        status: status.to_owned(),
        detail,
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn concise_error(error: &str) -> String {
    let compact = error.lines().next().unwrap_or(error).trim();
    if compact.chars().count() > 180 {
        let prefix = compact.chars().take(177).collect::<String>();
        format!("{prefix}...")
    } else if compact.is_empty() {
        "unknown error".to_owned()
    } else {
        compact.to_owned()
    }
}

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

fn local_stt_transcribe_blocking(
    audio_base64: String,
    language: Option<String>,
) -> Result<Value, String> {
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
    let result = run_whisper_cli(&path, language.as_deref());
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

fn run_whisper_cli(path: &Path, language: Option<&str>) -> Result<String, String> {
    let model = whisper_model_path()?;
    let language = whisper_language(language, &model);
    let library_path = whisper_library_path_env();
    let mut last_error = String::new();

    for binary in whisper_cli_candidates() {
        let mut command = Command::new(&binary);
        command
            .arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(path)
            .arg("-l")
            .arg(&language)
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
        candidates.push(home.join(".local/share/cadis/whisper-models/ggml-base.bin"));
        candidates.push(home.join(".local/share/cadis/whisper-models/ggml-base.en.bin"));
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
        "whisper model not found; set CADIS_WHISPER_MODEL or install ggml-base.bin under ~/.local/share/cadis/whisper-models ({searched})"
    ))
}

fn whisper_language(language: Option<&str>, model: &Path) -> String {
    let requested = env::var("CADIS_WHISPER_LANGUAGE")
        .ok()
        .or_else(|| env::var("WHISPER_LANGUAGE").ok())
        .or_else(|| language.map(str::to_owned))
        .and_then(|value| normalize_whisper_language(&value))
        .unwrap_or_else(|| "auto".to_owned());

    if is_english_only_whisper_model(model) && requested != "en" {
        "en".to_owned()
    } else {
        requested
    }
}

fn normalize_whisper_language(language: &str) -> Option<String> {
    let normalized = language.trim().to_lowercase().replace('_', "-");
    if normalized.is_empty() {
        return None;
    }
    if normalized == "auto" {
        return Some(normalized);
    }
    let base = normalized.split('-').next().unwrap_or(&normalized);
    if base == "in" {
        return Some("id".to_owned());
    }
    if base.len() >= 2 {
        return Some(base.to_owned());
    }
    None
}

fn is_english_only_whisper_model(model: &Path) -> bool {
    model
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(|file_name| file_name.contains(".en."))
        .unwrap_or(false)
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
    resolve_program_path(program).is_some()
}

fn resolve_candidate_path(candidate: &Path) -> Option<PathBuf> {
    if candidate.components().count() > 1 {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        return None;
    }
    candidate.to_str().and_then(resolve_program_path)
}

fn resolve_program_path(program: &str) -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v \"$1\"")
        .arg("sh")
        .arg(program)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
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

impl CadisSubscriptionState {
    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    fn replace_active_subscription(&self, stream: DaemonStream) -> Result<(), String> {
        let mut active = self
            .stream
            .lock()
            .map_err(|_| "CADIS subscription state lock was poisoned".to_owned())?;
        if let Some(existing) = active.take() {
            let _ = existing.shutdown(Shutdown::Both);
        }
        *active = Some(stream);
        Ok(())
    }

    fn close_active_subscription(&self) -> Result<(), String> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let stream = self
            .stream
            .lock()
            .map_err(|_| "CADIS subscription state lock was poisoned".to_owned())?
            .take();
        if let Some(stream) = stream {
            let _ = stream.shutdown(Shutdown::Both);
        }
        Ok(())
    }

    fn clear_active_subscription_if_current(&self, generation: u64) {
        if !self.is_current(generation) {
            return;
        }
        if let Ok(mut active) = self.stream.lock() {
            active.take();
        }
    }
}

fn start_cadis_event_subscription(
    app: tauri::AppHandle,
    state: Arc<CadisSubscriptionState>,
    transport: &DaemonTransport,
    request: Value,
) -> Result<(), String> {
    let mut stream = connect_daemon(transport)?;

    serde_json::to_writer(&mut stream, &request)
        .map_err(|error| format!("could not encode CADIS subscription request: {error}"))?;
    stream
        .write_all(b"\n")
        .map_err(|error| format!("could not send CADIS subscription request: {error}"))?;

    let active_stream = stream
        .try_clone()
        .map_err(|error| format!("could not track CADIS subscription socket: {error}"))?;
    let generation = state.next_generation();
    state.replace_active_subscription(active_stream)?;

    thread::spawn(move || {
        let result = read_subscription_frames(stream, |frame| {
            app.emit(CADIS_FRAME_EVENT, frame)
                .map_err(|error| io::Error::other(error.to_string()))
        });

        if state.is_current(generation) {
            state.clear_active_subscription_if_current(generation);
            let error = result.err().map(|error| error.to_string());
            let _ = app.emit(
                CADIS_SUBSCRIPTION_CLOSED_EVENT,
                CadisSubscriptionClosed { generation, error },
            );
        }
    });

    Ok(())
}

fn read_subscription_frames<F>(stream: DaemonStream, mut emit: F) -> io::Result<()>
where
    F: FnMut(Value) -> io::Result<()>,
{
    let reader = BufReader::new(stream);

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
                    "cadisd returned invalid subscription JSON on line {}: {error}",
                    index + 1
                ),
            )
        })?;
        emit(value)?;
    }

    Ok(())
}

fn send_cadis_request(transport: &DaemonTransport, request: Value) -> io::Result<Vec<Value>> {
    let mut stream = connect_daemon(transport).map_err(|msg| io::Error::new(io::ErrorKind::ConnectionRefused, msg))?;

    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;

    read_json_lines(stream)
}

fn read_json_lines(stream: DaemonStream) -> io::Result<Vec<Value>> {
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

/// Resolved daemon transport: either a Unix socket path or a TCP address.
#[derive(Debug)]
enum DaemonTransport {
    #[cfg(unix)]
    Socket(PathBuf),
    Tcp(String),
}

fn connect_daemon(transport: &DaemonTransport) -> Result<DaemonStream, String> {
    match transport {
        #[cfg(unix)]
        DaemonTransport::Socket(path) => UnixStream::connect(path)
            .map(DaemonStream::Unix)
            .map_err(|e| format!("could not connect to cadisd at {}: {e}", path.display())),
        DaemonTransport::Tcp(addr) => TcpStream::connect(addr)
            .map(DaemonStream::Tcp)
            .map_err(|e| format!("could not connect to cadisd at tcp://{addr}: {e}")),
    }
}

fn discover_transport(explicit_socket: Option<String>) -> Result<DaemonTransport, String> {
    let env = DiscoveryEnv::from_process();
    discover_transport_with_env(explicit_socket, &env)
}

fn discover_transport_with_env(
    explicit_socket: Option<String>,
    env: &DiscoveryEnv,
) -> Result<DaemonTransport, String> {
    // TCP port env takes highest priority (Windows default path).
    if let Some(port) = non_empty(env.cadis_tcp_port.clone()) {
        let port: u16 = port
            .parse()
            .map_err(|e| format!("CADIS_TCP_PORT is not a valid port: {e}"))?;
        return Ok(DaemonTransport::Tcp(format!("127.0.0.1:{port}")));
    }

    // On Unix, try socket paths before falling back to TCP.
    #[cfg(unix)]
    {
        if let Some(path) = non_empty(explicit_socket) {
            return expand_home(&path, env)
                .map(DaemonTransport::Socket)
                .map_err(|e| e.to_string());
        }

        if let Some(path) = non_empty(env.cadis_hud_socket.clone()) {
            return expand_home(&path, env)
                .map(DaemonTransport::Socket)
                .map_err(|e| e.to_string());
        }

        if let Some(path) = non_empty(env.cadis_socket.clone()) {
            return expand_home(&path, env)
                .map(DaemonTransport::Socket)
                .map_err(|e| e.to_string());
        }

        if let Some(path) = config_socket_path(env)? {
            return expand_home(&path, env)
                .map(DaemonTransport::Socket)
                .map_err(|e| e.to_string());
        }

        if let Some(runtime_dir) = non_empty(env.xdg_runtime_dir.clone()) {
            return Ok(DaemonTransport::Socket(
                PathBuf::from(runtime_dir).join("cadis").join("cadisd.sock"),
            ));
        }

        if let Some(home) = env.home.as_ref() {
            return Ok(DaemonTransport::Socket(
                home.join(CADIS_SOCKET_RELATIVE_PATH),
            ));
        }
    }

    // Non-Unix or no socket path resolved: use TCP from config or default.
    #[cfg(not(unix))]
    let _ = explicit_socket;

    if let Some(addr) = config_tcp_address(env)? {
        return Ok(DaemonTransport::Tcp(addr));
    }

    Ok(DaemonTransport::Tcp(DEFAULT_TCP_ADDRESS.to_owned()))
}

#[cfg(unix)]
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

#[cfg(unix)]
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
    cadis_tcp_port: Option<String>,
    cadis_hud_socket: Option<String>,
    cadis_socket: Option<String>,
    home: Option<PathBuf>,
    #[cfg(unix)]
    xdg_runtime_dir: Option<String>,
}

impl DiscoveryEnv {
    fn from_process() -> Self {
        Self {
            cadis_tcp_port: env::var("CADIS_TCP_PORT").ok(),
            cadis_hud_socket: env::var("CADIS_HUD_SOCKET").ok(),
            cadis_socket: env::var("CADIS_SOCKET").ok(),
            home: env::var_os("HOME").map(PathBuf::from),
            #[cfg(unix)]
            xdg_runtime_dir: env::var("XDG_RUNTIME_DIR").ok(),
        }
    }
}

fn config_tcp_address(env: &DiscoveryEnv) -> Result<Option<String>, String> {
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
        .get("tcp_address")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .and_then(|v| non_empty(Some(v))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::net::UnixListener;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn discovery_prefers_explicit_socket_path() {
        let env = DiscoveryEnv {
            cadis_tcp_port: None,
            cadis_hud_socket: Some("/tmp/hud.sock".to_owned()),
            cadis_socket: Some("/tmp/cadis.sock".to_owned()),
            home: Some(PathBuf::from("/home/cadis")),
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
        };

        let transport =
            discover_transport_with_env(Some("~/explicit.sock".to_owned()), &env).unwrap();

        match transport {
            DaemonTransport::Socket(path) => {
                assert_eq!(path, PathBuf::from("/home/cadis/explicit.sock"));
            }
            _ => panic!("expected Socket transport"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn discovery_prefers_hud_env_over_generic_env() {
        let env = DiscoveryEnv {
            cadis_tcp_port: None,
            cadis_hud_socket: Some("/tmp/hud.sock".to_owned()),
            cadis_socket: Some("/tmp/cadis.sock".to_owned()),
            home: Some(PathBuf::from("/home/cadis")),
            xdg_runtime_dir: None,
        };

        let transport = discover_transport_with_env(None, &env).unwrap();

        match transport {
            DaemonTransport::Socket(path) => {
                assert_eq!(path, PathBuf::from("/tmp/hud.sock"));
            }
            _ => panic!("expected Socket transport"),
        }
    }

    #[cfg(unix)]
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
            cadis_tcp_port: None,
            home: Some(home.clone()),
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
            ..DiscoveryEnv::default()
        };

        let transport = discover_transport_with_env(None, &env).unwrap();

        match transport {
            DaemonTransport::Socket(path) => {
                assert_eq!(path, home.join(".cadis/custom.sock"));
            }
            _ => panic!("expected Socket transport"),
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discovery_uses_xdg_runtime_dir_before_home_default() {
        let env = DiscoveryEnv {
            cadis_tcp_port: None,
            home: Some(PathBuf::from("/home/cadis")),
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
            ..DiscoveryEnv::default()
        };

        let transport = discover_transport_with_env(None, &env).unwrap();

        match transport {
            DaemonTransport::Socket(path) => {
                assert_eq!(path, PathBuf::from("/run/user/1000/cadis/cadisd.sock"));
            }
            _ => panic!("expected Socket transport"),
        }
    }

    #[test]
    fn discovery_tcp_port_env_takes_priority() {
        let env = DiscoveryEnv {
            cadis_tcp_port: Some("9999".to_owned()),
            cadis_hud_socket: Some("/tmp/hud.sock".to_owned()),
            cadis_socket: Some("/tmp/cadis.sock".to_owned()),
            home: Some(PathBuf::from("/home/cadis")),
            #[cfg(unix)]
            xdg_runtime_dir: Some("/run/user/1000".to_owned()),
        };

        let transport = discover_transport_with_env(None, &env).unwrap();

        match transport {
            DaemonTransport::Tcp(addr) => assert_eq!(addr, "127.0.0.1:9999"),
            #[cfg(unix)]
            _ => panic!("expected Tcp transport"),
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn discovery_defaults_to_tcp_on_non_unix() {
        let env = DiscoveryEnv {
            cadis_tcp_port: None,
            cadis_hud_socket: None,
            cadis_socket: None,
            home: None,
        };

        let transport = discover_transport_with_env(None, &env).unwrap();

        match transport {
            DaemonTransport::Tcp(addr) => assert_eq!(addr, DEFAULT_TCP_ADDRESS),
        }
    }

    #[cfg(unix)]
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

        let transport = DaemonTransport::Socket(socket_path);
        let frames = send_cadis_request(
            &transport,
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

    #[cfg(unix)]
    #[test]
    fn subscription_reader_emits_each_json_line() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            writer
                .write_all(
                    b"{\"frame\":\"response\",\"payload\":{\"type\":\"request.accepted\"}}\n\
                      \n\
                      {\"frame\":\"event\",\"payload\":{\"event_id\":\"evt_1\",\"type\":\"agent.list.response\",\"payload\":{\"agents\":[]}}}\n",
                )
                .unwrap();
        });

        let mut frames = Vec::new();
        read_subscription_frames(DaemonStream::Unix(reader), |frame| {
            frames.push(frame);
            Ok(())
        })
        .unwrap();

        server.join().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["frame"], "response");
        assert_eq!(frames[1]["payload"]["event_id"], "evt_1");
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("cadis-hud-test-{}-{nanos}", std::process::id()))
    }
}
