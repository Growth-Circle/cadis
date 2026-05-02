//! Voice, TTS providers, speech decisions, and voice doctor utilities.

use super::*;
use std::io::Write as _;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VoicePreflightRecord {
    pub(crate) surface: String,
    pub(crate) status: String,
    pub(crate) summary: String,
    pub(crate) checked_at: Timestamp,
    pub(crate) checks: Vec<VoiceDoctorCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VoiceRuntimePreferences {
    pub(crate) enabled: bool,
    pub(crate) provider: String,
    pub(crate) voice_id: String,
    pub(crate) stt_language: String,
    pub(crate) rate: i16,
    pub(crate) pitch: i16,
    pub(crate) volume: i16,
    pub(crate) auto_speak: bool,
    pub(crate) max_spoken_chars: usize,
}

impl VoiceRuntimePreferences {
    pub(crate) fn from_options(options: &serde_json::Value) -> Self {
        let voice = options.get("voice").and_then(serde_json::Value::as_object);

        Self {
            enabled: voice
                .and_then(|voice| voice.get("enabled"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            provider: voice
                .and_then(|voice| voice.get("provider"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_voice_config_string)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "edge".to_owned()),
            voice_id: voice
                .and_then(|voice| voice.get("voice_id"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_voice_config_string)
                .unwrap_or_else(|| "id-ID-GadisNeural".to_owned()),
            stt_language: voice
                .and_then(|voice| voice.get("stt_language"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_voice_config_string)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "auto".to_owned()),
            rate: voice
                .and_then(|voice| voice.get("rate"))
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i16::try_from(value).ok())
                .map(clamp_voice_adjustment)
                .unwrap_or_default(),
            pitch: voice
                .and_then(|voice| voice.get("pitch"))
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i16::try_from(value).ok())
                .map(clamp_voice_adjustment)
                .unwrap_or_default(),
            volume: voice
                .and_then(|voice| voice.get("volume"))
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i16::try_from(value).ok())
                .map(clamp_voice_adjustment)
                .unwrap_or_default(),
            auto_speak: voice
                .and_then(|voice| voice.get("auto_speak"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            max_spoken_chars: voice
                .and_then(|voice| voice.get("max_spoken_chars"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .unwrap_or(800),
        }
    }

    pub(crate) fn from_preview(
        options: &serde_json::Value,
        prefs: Option<VoicePreferences>,
    ) -> Self {
        let mut runtime_prefs = Self::from_options(options);
        if let Some(prefs) = prefs {
            runtime_prefs.voice_id = normalize_voice_config_string(&prefs.voice_id);
            runtime_prefs.rate = clamp_voice_adjustment(prefs.rate);
            runtime_prefs.pitch = clamp_voice_adjustment(prefs.pitch);
            runtime_prefs.volume = clamp_voice_adjustment(prefs.volume);
        }
        runtime_prefs
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TtsProviderKind {
    Edge,
    OpenAi,
    System,
    Stub,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StubbedTtsProvider {
    pub(crate) kind: TtsProviderKind,
    pub(crate) configured_id: String,
}

impl StubbedTtsProvider {
    pub(crate) fn new(provider: &str) -> Self {
        Self {
            kind: tts_provider_kind(provider),
            configured_id: normalize_voice_config_string(provider),
        }
    }
}

impl TtsProvider for StubbedTtsProvider {
    fn id(&self) -> &'static str {
        match self.kind {
            TtsProviderKind::Edge => "edge",
            TtsProviderKind::OpenAi => "openai",
            TtsProviderKind::System => "system",
            TtsProviderKind::Stub => "stub",
            TtsProviderKind::Unsupported => "unsupported",
        }
    }

    fn label(&self) -> &'static str {
        match self.kind {
            TtsProviderKind::Edge => "Edge TTS daemon stub",
            TtsProviderKind::OpenAi => "OpenAI TTS daemon stub",
            TtsProviderKind::System => "System speech daemon stub",
            TtsProviderKind::Stub => "Deterministic test TTS stub",
            TtsProviderKind::Unsupported => "Unsupported TTS provider",
        }
    }

    fn supported_voices(&self) -> Vec<TtsVoice> {
        curated_tts_voices()
    }

    fn speak(&mut self, request: TtsRequest<'_>) -> Result<TtsOutput, TtsError> {
        if self.kind == TtsProviderKind::Unsupported {
            return Err(TtsError::new(
                "unsupported_tts_provider",
                format!("unsupported TTS provider '{}'", self.configured_id),
                false,
            ));
        }
        Ok(TtsOutput {
            provider: self.id().to_owned(),
            voice_id: request.voice_id.to_owned(),
            spoken_chars: request.text.chars().count(),
            audio_path: None,
        })
    }

    fn stop(&mut self) -> Result<(), TtsError> {
        if self.kind == TtsProviderKind::Unsupported {
            return Err(TtsError::new(
                "unsupported_tts_provider",
                format!("unsupported TTS provider '{}'", self.configured_id),
                false,
            ));
        }
        Ok(())
    }
}

/// Daemon-owned Edge TTS provider that calls the `edge-tts` Python CLI.
pub(crate) struct EdgeTtsProvider;

impl TtsProvider for EdgeTtsProvider {
    fn id(&self) -> &'static str {
        "edge"
    }

    fn label(&self) -> &'static str {
        "Edge TTS (daemon subprocess)"
    }

    fn supported_voices(&self) -> Vec<TtsVoice> {
        curated_tts_voices()
    }

    fn speak(&mut self, request: TtsRequest<'_>) -> Result<TtsOutput, TtsError> {
        // Validate voice_id: only allow alphanumeric, dash, and dot.
        if request.voice_id.is_empty()
            || !request
                .voice_id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '.')
        {
            return Err(TtsError::new(
                "invalid_voice_id",
                "voice_id must be non-empty and contain only alphanumeric, dash, or dot characters",
                false,
            ));
        }

        // Truncate text to prevent excessively long subprocess arguments.
        pub(crate) const MAX_TTS_TEXT_CHARS: usize = 5000;
        let text = if request.text.chars().count() > MAX_TTS_TEXT_CHARS {
            truncate_to_utf8_boundary(request.text, MAX_TTS_TEXT_CHARS).0
        } else {
            request.text
        };

        let temp_dir = std::env::temp_dir().join("cadis-edge-tts");
        let _ = fs::create_dir_all(&temp_dir);
        // Use PID + nanos for a unique path per invocation.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let audio_path = temp_dir.join(format!("cadis-tts-{}-{nanos}.mp3", std::process::id()));

        let rate_arg = format!("{:+}%", request.rate);
        let pitch_arg = format!("{:+}Hz", request.pitch);
        let volume_arg = format!("{:+}%", request.volume);

        let output = Command::new("edge-tts")
            .args(["--voice", request.voice_id])
            .args(["--rate", &rate_arg])
            .args(["--pitch", &pitch_arg])
            .args(["--volume", &volume_arg])
            .args(["--text", text])
            .arg("--write-media")
            .arg(&audio_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(result) if result.status.success() => Ok(TtsOutput {
                provider: "edge".to_owned(),
                voice_id: request.voice_id.to_owned(),
                spoken_chars: request.text.chars().count(),
                audio_path: Some(audio_path),
            }),
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(TtsError::new(
                    "edge_tts_failed",
                    format!(
                        "edge-tts exited with code {:?}: {}",
                        result.status.code(),
                        stderr.trim()
                    ),
                    true,
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(TtsError::new(
                "edge_tts_not_found",
                "edge-tts binary is not installed; install with: pip install edge-tts",
                false,
            )),
            Err(error) => Err(TtsError::new(
                "edge_tts_spawn_failed",
                error.to_string(),
                true,
            )),
        }
    }

    fn stop(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

/// Daemon-owned System TTS provider that uses platform-native speech synthesis.
///
/// On Linux, uses `espeak` or `espeak-ng` via subprocess.
/// On macOS, uses `say` via subprocess.
/// On Windows, uses PowerShell `Add-Type -AssemblyName System.Speech` via subprocess.
pub(crate) struct SystemTtsProvider;

impl TtsProvider for SystemTtsProvider {
    fn id(&self) -> &'static str {
        "system"
    }

    fn label(&self) -> &'static str {
        "System TTS (native speech daemon)"
    }

    fn supported_voices(&self) -> Vec<TtsVoice> {
        vec![TtsVoice {
            id: "default",
            label: "System default voice",
            locale: "en-US",
            gender: "Neutral",
        }]
    }

    fn speak(&mut self, request: TtsRequest<'_>) -> Result<TtsOutput, TtsError> {
        let text = request.text.trim();
        if text.is_empty() {
            return Err(TtsError::new(
                "empty_text",
                "cannot speak empty text",
                false,
            ));
        }

        let platform = SystemTtsPlatform::current();
        let binary = system_tts_binary_for_platform(platform)
            .unwrap_or_else(|| default_system_tts_binary(platform));
        let plan = system_tts_command_plan(platform, binary, text, request.voice_id, request.rate);
        let result = run_system_tts_command(&plan);

        match result {
            Ok(output) if output.status.success() => Ok(TtsOutput {
                provider: "system".to_owned(),
                voice_id: plan.voice_id,
                spoken_chars: plan.spoken_chars,
                audio_path: None,
            }),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(TtsError::new(
                    "system_tts_failed",
                    format!("system TTS failed: {}", redact(stderr.trim())),
                    true,
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(TtsError::new(
                "system_tts_not_found",
                format!(
                    "{} is not installed; system TTS requires {}",
                    plan.binary,
                    system_tts_binary_requirement(platform)
                ),
                false,
            )),
            Err(error) => Err(TtsError::new(
                "system_tts_spawn_failed",
                error.to_string(),
                true,
            )),
        }
    }

    fn stop(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

/// Daemon-owned OpenAI TTS provider that calls the OpenAI speech API.
pub(crate) struct OpenAiTtsProvider {
    api_key: String,
    base_url: String,
}

const OPENAI_TTS_ERROR_BODY_LIMIT_BYTES: usize = 2 * 1024;
const OPENAI_TTS_MIN_MP3_BYTES: u64 = 16;

impl OpenAiTtsProvider {
    pub(crate) fn new(api_key: String) -> Self {
        let base_url = std::env::var("CADIS_OPENAI_BASE_URL")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        Self { api_key, base_url }
    }

    fn openai_voice_id(voice_id: &str) -> &'static str {
        match voice_id {
            "alloy" => "alloy",
            "echo" => "echo",
            "fable" => "fable",
            "onyx" => "onyx",
            "nova" => "nova",
            "shimmer" => "shimmer",
            _ => "alloy",
        }
    }
}

impl TtsProvider for OpenAiTtsProvider {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn label(&self) -> &'static str {
        "OpenAI TTS (daemon HTTP)"
    }

    fn supported_voices(&self) -> Vec<TtsVoice> {
        vec![
            TtsVoice {
                id: "alloy",
                label: "Alloy (Neutral)",
                locale: "en-US",
                gender: "Neutral",
            },
            TtsVoice {
                id: "echo",
                label: "Echo (Male)",
                locale: "en-US",
                gender: "Male",
            },
            TtsVoice {
                id: "fable",
                label: "Fable (Neutral)",
                locale: "en-US",
                gender: "Neutral",
            },
            TtsVoice {
                id: "onyx",
                label: "Onyx (Male)",
                locale: "en-US",
                gender: "Male",
            },
            TtsVoice {
                id: "nova",
                label: "Nova (Female)",
                locale: "en-US",
                gender: "Female",
            },
            TtsVoice {
                id: "shimmer",
                label: "Shimmer (Female)",
                locale: "en-US",
                gender: "Female",
            },
        ]
    }

    fn speak(&mut self, request: TtsRequest<'_>) -> Result<TtsOutput, TtsError> {
        let voice = Self::openai_voice_id(request.voice_id);

        pub(crate) const MAX_OPENAI_TTS_CHARS: usize = 4096;
        let text = if request.text.chars().count() > MAX_OPENAI_TTS_CHARS {
            truncate_to_utf8_boundary(request.text, MAX_OPENAI_TTS_CHARS).0
        } else {
            request.text
        };

        let speed = 1.0 + (f64::from(request.rate) / 100.0);
        let speed = speed.clamp(0.25, 4.0);

        let body = serde_json::json!({
            "model": "tts-1",
            "voice": voice,
            "input": text,
            "speed": speed,
            "response_format": "mp3"
        });

        let temp_dir = std::env::temp_dir().join("cadis-openai-tts");
        let _ = fs::create_dir_all(&temp_dir);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let audio_path = temp_dir.join(format!(
            "cadis-tts-openai-{}-{nanos}.mp3",
            std::process::id()
        ));

        let url = format!("{}/audio/speech", self.base_url.trim_end_matches('/'));

        let request_body = serde_json::to_string(&body).unwrap_or_default();
        let output = Command::new("curl")
            .args(openai_tts_curl_args(
                &url,
                &request_body,
                &audio_path,
                &self.api_key,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(result) if result.status.success() => {
                let http_status = parse_curl_http_status(&result.stdout);
                if let Some(status) = http_status.filter(|status| !(200..300).contains(status)) {
                    let _ = fs::remove_file(&audio_path);
                    return Err(TtsError::new(
                        "openai_tts_failed",
                        openai_tts_failure_message(
                            Some(status),
                            result.status.code(),
                            &result.stderr,
                            &audio_path,
                            &self.api_key,
                        ),
                        true,
                    ));
                }
                if let Err(error) = validate_openai_tts_audio_file(&audio_path, &self.api_key) {
                    let _ = fs::remove_file(&audio_path);
                    return Err(error);
                }
                Ok(TtsOutput {
                    provider: "openai".to_owned(),
                    voice_id: voice.to_owned(),
                    spoken_chars: request.text.chars().count(),
                    audio_path: Some(audio_path),
                })
            }
            Ok(result) => {
                let http_status = parse_curl_http_status(&result.stdout);
                let message = openai_tts_failure_message(
                    http_status,
                    result.status.code(),
                    &result.stderr,
                    &audio_path,
                    &self.api_key,
                );
                let _ = fs::remove_file(&audio_path);
                Err(TtsError::new("openai_tts_failed", message, true))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(TtsError::new(
                "curl_not_found",
                "curl is not installed; OpenAI TTS requires curl for HTTP requests",
                false,
            )),
            Err(error) => Err(TtsError::new(
                "openai_tts_spawn_failed",
                error.to_string(),
                true,
            )),
        }
    }

    fn stop(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

const SYSTEM_DEFAULT_VOICE_ID: &str = "default";
const MAX_SYSTEM_TTS_CHARS: usize = 2000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SystemTtsPlatform {
    Linux,
    Macos,
    Windows,
}

impl SystemTtsPlatform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SystemTtsCommandPlan {
    binary: &'static str,
    args: Vec<String>,
    stdin_text: Option<String>,
    voice_id: String,
    spoken_chars: usize,
}

fn system_tts_command_plan(
    platform: SystemTtsPlatform,
    binary: &'static str,
    text: &str,
    voice_id: &str,
    rate: i16,
) -> SystemTtsCommandPlan {
    let text = if text.chars().count() > MAX_SYSTEM_TTS_CHARS {
        truncate_to_utf8_boundary(text, MAX_SYSTEM_TTS_CHARS).0
    } else {
        text
    };
    let spoken_chars = text.chars().count();
    let voice_id = system_tts_voice_id(voice_id).to_owned();
    let args = match platform {
        SystemTtsPlatform::Macos => vec![
            "-r".to_owned(),
            words_per_minute(rate).to_string(),
            text.to_owned(),
        ],
        SystemTtsPlatform::Windows => vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-EncodedCommand".to_owned(),
            base64_encode_utf16le(&system_tts_powershell_script(rate)),
        ],
        SystemTtsPlatform::Linux => vec![
            "-s".to_owned(),
            words_per_minute(rate).to_string(),
            text.to_owned(),
        ],
    };

    SystemTtsCommandPlan {
        binary,
        args,
        stdin_text: (platform == SystemTtsPlatform::Windows).then(|| text.to_owned()),
        voice_id,
        spoken_chars,
    }
}

fn run_system_tts_command(plan: &SystemTtsCommandPlan) -> io::Result<std::process::Output> {
    let mut child = Command::new(plan.binary)
        .args(&plan.args)
        .stdin(if plan.stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(text) = &plan.stdin_text {
        let mut stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "system TTS subprocess stdin was not available",
            )
        })?;
        stdin.write_all(text.as_bytes())?;
    }

    child.wait_with_output()
}

fn system_tts_voice_id(_voice_id: &str) -> &'static str {
    SYSTEM_DEFAULT_VOICE_ID
}

fn system_tts_powershell_script(rate: i16) -> String {
    format!(
        "Add-Type -AssemblyName System.Speech\n\
         $synth = [System.Speech.Synthesis.SpeechSynthesizer]::new()\n\
         $synth.Rate = {}\n\
         $text = [Console]::In.ReadToEnd()\n\
         if ($text.Length -gt 0) {{ $synth.Speak($text) }}\n\
         $synth.Dispose()\n",
        ps_rate(rate)
    )
}

fn base64_encode_utf16le(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len() * 2);
    for code_unit in value.encode_utf16() {
        bytes.extend_from_slice(&code_unit.to_le_bytes());
    }
    base64_encode(&bytes)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);

        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

fn words_per_minute(rate: i16) -> u32 {
    let base: f64 = 175.0;
    let adjusted = base * (1.0 + f64::from(rate) / 100.0);
    adjusted.clamp(80.0, 450.0) as u32
}

fn ps_rate(rate: i16) -> i32 {
    ((f64::from(rate) / 10.0).round() as i32).clamp(-10, 10)
}

fn system_tts_binary_candidates(platform: SystemTtsPlatform) -> &'static [&'static str] {
    match platform {
        SystemTtsPlatform::Macos => &["say"],
        SystemTtsPlatform::Windows => &["powershell"],
        SystemTtsPlatform::Linux => &["espeak", "espeak-ng"],
    }
}

fn default_system_tts_binary(platform: SystemTtsPlatform) -> &'static str {
    system_tts_binary_candidates(platform)[0]
}

fn system_tts_binary_requirement(platform: SystemTtsPlatform) -> String {
    system_tts_binary_candidates(platform).join(" or ")
}

fn system_tts_binary_for_platform(platform: SystemTtsPlatform) -> Option<&'static str> {
    system_tts_binary_candidates(platform)
        .iter()
        .copied()
        .find(|binary| command_exists_on_path(binary, platform))
}

fn command_exists_on_path(binary: &str, platform: SystemTtsPlatform) -> bool {
    let Some(path_value) = std::env::var_os("PATH") else {
        return false;
    };
    let pathext_value = std::env::var_os("PATHEXT");
    command_exists_in_path(binary, platform, &path_value, pathext_value.as_deref())
}

fn command_exists_in_path(
    binary: &str,
    platform: SystemTtsPlatform,
    path_value: &std::ffi::OsStr,
    pathext_value: Option<&std::ffi::OsStr>,
) -> bool {
    std::env::split_paths(path_value).any(|dir| {
        command_path_candidates(binary, platform, &dir, pathext_value)
            .iter()
            .any(|candidate| candidate.is_file())
    })
}

fn command_path_candidates(
    binary: &str,
    platform: SystemTtsPlatform,
    dir: &Path,
    pathext_value: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    if platform != SystemTtsPlatform::Windows || Path::new(binary).extension().is_some() {
        return vec![dir.join(binary)];
    }

    let mut candidates = vec![dir.join(binary)];
    for extension in windows_pathexts(pathext_value) {
        candidates.push(dir.join(format!("{binary}{extension}")));
    }
    candidates
}

fn windows_pathexts(pathext_value: Option<&std::ffi::OsStr>) -> Vec<String> {
    if let Some(value) = pathext_value {
        let extensions = value
            .to_string_lossy()
            .split(';')
            .map(str::trim)
            .filter(|extension| !extension.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !extensions.is_empty() {
            return extensions;
        }
    }
    [".COM", ".EXE", ".BAT", ".CMD"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn system_tts_available() -> bool {
    system_tts_binary_for_platform(SystemTtsPlatform::current()).is_some()
}

fn openai_tts_curl_args(
    url: &str,
    request_body: &str,
    audio_path: &Path,
    api_key: &str,
) -> Vec<String> {
    vec![
        "--silent".to_owned(),
        "--show-error".to_owned(),
        "--fail-with-body".to_owned(),
        "--request".to_owned(),
        "POST".to_owned(),
        url.to_owned(),
        "--header".to_owned(),
        "Content-Type: application/json".to_owned(),
        "--header".to_owned(),
        format!("Authorization: Bearer {api_key}"),
        "--data".to_owned(),
        request_body.to_owned(),
        "--output".to_owned(),
        audio_path.to_string_lossy().into_owned(),
        "--write-out".to_owned(),
        "%{http_code}".to_owned(),
    ]
}

fn parse_curl_http_status(stdout: &[u8]) -> Option<u16> {
    let status = String::from_utf8_lossy(stdout).trim().parse::<u16>().ok()?;
    (status > 0).then_some(status)
}

fn validate_openai_tts_audio_file(audio_path: &Path, api_key: &str) -> Result<u64, TtsError> {
    let file_size = fs::metadata(audio_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if file_size == 0 {
        return Err(TtsError::new(
            "openai_tts_empty_response",
            "OpenAI TTS returned an empty audio file",
            true,
        ));
    }

    let mut file = fs::File::open(audio_path).map_err(|error| {
        TtsError::new(
            "openai_tts_audio_read_failed",
            format!(
                "OpenAI TTS audio file could not be read: {}",
                redact_openai_tts_context(&error.to_string(), api_key)
            ),
            true,
        )
    })?;
    let mut header = [0_u8; 16];
    let header_len = file.read(&mut header).map_err(|error| {
        TtsError::new(
            "openai_tts_audio_read_failed",
            format!(
                "OpenAI TTS audio file could not be read: {}",
                redact_openai_tts_context(&error.to_string(), api_key)
            ),
            true,
        )
    })?;

    if file_size < OPENAI_TTS_MIN_MP3_BYTES || !looks_like_mp3_header(&header[..header_len]) {
        let mut message =
            format!("OpenAI TTS response was not a plausible MP3 ({file_size} bytes)");
        if let Some(body) = read_openai_tts_body_context(audio_path, api_key) {
            message.push_str(": response body: ");
            message.push_str(&body);
        }
        return Err(TtsError::new(
            "openai_tts_invalid_audio",
            redact_openai_tts_context(&message, api_key),
            true,
        ));
    }

    Ok(file_size)
}

fn looks_like_mp3_header(header: &[u8]) -> bool {
    header.starts_with(b"ID3")
        || header
            .get(..2)
            .is_some_and(|bytes| bytes[0] == 0xff && (bytes[1] & 0xe0) == 0xe0)
}

fn openai_tts_failure_message(
    http_status: Option<u16>,
    exit_code: Option<i32>,
    stderr: &[u8],
    body_path: &Path,
    api_key: &str,
) -> String {
    let status = match (http_status, exit_code) {
        (Some(status), Some(code)) => format!(" (HTTP {status}, curl exit {code})"),
        (Some(status), None) => format!(" (HTTP {status})"),
        (None, Some(code)) => format!(" (curl exit {code})"),
        (None, None) => String::new(),
    };
    let context = openai_tts_error_context(stderr, body_path, api_key)
        .unwrap_or_else(|| "no response details".to_owned());
    redact_openai_tts_context(
        &format!("OpenAI TTS request failed{status}: {context}"),
        api_key,
    )
}

fn openai_tts_error_context(stderr: &[u8], body_path: &Path, api_key: &str) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(stderr) = sanitize_openai_tts_context(&String::from_utf8_lossy(stderr), api_key) {
        parts.push(format!("curl stderr: {stderr}"));
    }
    if let Some(body) = read_openai_tts_body_context(body_path, api_key) {
        parts.push(format!("response body: {body}"));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn read_openai_tts_body_context(body_path: &Path, api_key: &str) -> Option<String> {
    let mut file = fs::File::open(body_path).ok()?;
    let mut buffer = vec![0_u8; OPENAI_TTS_ERROR_BODY_LIMIT_BYTES + 1];
    let bytes_read = file.read(&mut buffer).ok()?;
    if bytes_read == 0 {
        return None;
    }

    let truncated = bytes_read > OPENAI_TTS_ERROR_BODY_LIMIT_BYTES;
    buffer.truncate(bytes_read.min(OPENAI_TTS_ERROR_BODY_LIMIT_BYTES));
    let mut context = sanitize_openai_tts_context(&String::from_utf8_lossy(&buffer), api_key)?;
    if truncated {
        context.push_str(" ...");
    }
    Some(context)
}

fn sanitize_openai_tts_context(input: &str, api_key: &str) -> Option<String> {
    let printable = input
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let compact = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    (!compact.is_empty()).then(|| redact_openai_tts_context(&compact, api_key))
}

fn redact_openai_tts_context(input: &str, api_key: &str) -> String {
    let redacted = redact(input);
    if api_key.is_empty() {
        redacted
    } else {
        redacted.replace(api_key, "[REDACTED]")
    }
}

fn openai_api_key_from_env() -> Option<String> {
    std::env::var("CADIS_OPENAI_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
        })
}

pub(crate) fn tts_provider_from_config(provider: &str) -> Box<dyn TtsProvider> {
    match provider {
        "edge" => Box::new(EdgeTtsProvider),
        "openai" => {
            if let Some(key) = openai_api_key_from_env() {
                Box::new(OpenAiTtsProvider::new(key))
            } else {
                Box::new(StubbedTtsProvider::new(provider))
            }
        }
        "system" => {
            if system_tts_available() {
                Box::new(SystemTtsProvider)
            } else {
                Box::new(StubbedTtsProvider::new(provider))
            }
        }
        _ => Box::new(StubbedTtsProvider::new(provider)),
    }
}

pub(crate) fn tts_provider_kind(provider: &str) -> TtsProviderKind {
    match provider {
        "edge" => TtsProviderKind::Edge,
        "openai" => TtsProviderKind::OpenAi,
        "system" => TtsProviderKind::System,
        "stub" => TtsProviderKind::Stub,
        _ => TtsProviderKind::Unsupported,
    }
}

pub(crate) fn curated_tts_voices() -> Vec<TtsVoice> {
    vec![
        TtsVoice {
            id: "id-ID-ArdiNeural",
            label: "Ardi (Indonesian, Male)",
            locale: "id-ID",
            gender: "Male",
        },
        TtsVoice {
            id: "id-ID-GadisNeural",
            label: "Gadis (Indonesian, Female)",
            locale: "id-ID",
            gender: "Female",
        },
        TtsVoice {
            id: "ms-MY-OsmanNeural",
            label: "Osman (Malay, Male)",
            locale: "ms-MY",
            gender: "Male",
        },
        TtsVoice {
            id: "ms-MY-YasminNeural",
            label: "Yasmin (Malay, Female)",
            locale: "ms-MY",
            gender: "Female",
        },
        TtsVoice {
            id: "en-US-AvaNeural",
            label: "Ava (US, Female)",
            locale: "en-US",
            gender: "Female",
        },
        TtsVoice {
            id: "en-US-AndrewNeural",
            label: "Andrew (US, Male)",
            locale: "en-US",
            gender: "Male",
        },
        TtsVoice {
            id: "en-US-EmmaNeural",
            label: "Emma (US, Female)",
            locale: "en-US",
            gender: "Female",
        },
        TtsVoice {
            id: "en-US-BrianNeural",
            label: "Brian (US, Male)",
            locale: "en-US",
            gender: "Male",
        },
        TtsVoice {
            id: "en-GB-SoniaNeural",
            label: "Sonia (GB, Female)",
            locale: "en-GB",
            gender: "Female",
        },
        TtsVoice {
            id: "en-GB-RyanNeural",
            label: "Ryan (GB, Male)",
            locale: "en-GB",
            gender: "Male",
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpeechMode {
    AutoSpeak,
    Preview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SpeechDecision {
    Speak,
    Blocked(&'static str),
    RequiresSummary(&'static str),
}

pub(crate) fn speech_decision(
    prefs: &VoiceRuntimePreferences,
    content_kind: ContentKind,
    text: &str,
    mode: SpeechMode,
) -> SpeechDecision {
    let text = text.trim();
    if text.is_empty() {
        return SpeechDecision::Blocked("empty_text");
    }

    if mode == SpeechMode::AutoSpeak {
        if !prefs.enabled {
            return SpeechDecision::Blocked("voice_disabled");
        }
        if !prefs.auto_speak {
            return SpeechDecision::Blocked("auto_speak_disabled");
        }
    }

    match content_kind {
        ContentKind::Code => return SpeechDecision::Blocked("code_not_speakable"),
        ContentKind::Diff => return SpeechDecision::Blocked("diff_not_speakable"),
        ContentKind::TerminalLog => return SpeechDecision::Blocked("terminal_log_not_speakable"),
        ContentKind::TestResult if text.chars().count() > prefs.max_spoken_chars => {
            return SpeechDecision::Blocked("long_tool_output_not_speakable");
        }
        ContentKind::TestResult if looks_like_raw_tool_output(text) => {
            return SpeechDecision::Blocked("raw_tool_output_not_speakable");
        }
        _ => {}
    }

    if text.chars().count() > prefs.max_spoken_chars {
        return SpeechDecision::RequiresSummary("content_exceeds_max_spoken_chars");
    }

    SpeechDecision::Speak
}

/// Truncates text to the first 2-3 sentences for spoken summary.
pub(crate) fn summarize_for_speech(text: &str) -> String {
    let mut end = 0;
    let mut sentences = 0;
    for (index, character) in text.char_indices() {
        if matches!(character, '.' | '!' | '?') {
            let next = text[index + character.len_utf8()..].chars().next();
            if next.is_none() || next.is_some_and(|c| c.is_whitespace()) {
                sentences += 1;
                end = index + character.len_utf8();
                if sentences >= 3 {
                    break;
                }
            }
        }
    }
    if end == 0 {
        text.split_whitespace()
            .take(30)
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        text[..end].trim().to_owned()
    }
}

/// Generates a short spoken risk summary for an approval request.
pub(crate) fn approval_risk_speech(record: &ApprovalRecord) -> String {
    let mut speech = format!("Approval needed: {}", record.tool_name);
    if let Some(command) = &record.command {
        let short = command.chars().take(60).collect::<String>();
        speech.push_str(&format!(", {short}"));
    }
    if let Some(workspace) = &record.workspace {
        let name = Path::new(workspace)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(workspace);
        speech.push_str(&format!(" in workspace {name}"));
    }
    speech
}

pub(crate) fn looks_like_raw_tool_output(text: &str) -> bool {
    let line_count = text.lines().count();
    line_count > 12
        || text.contains("```")
        || text.contains("diff --git")
        || text.contains("thread '")
        || text.contains("panicked at")
        || text.contains("error[E")
}

pub(crate) fn normalize_voice_checks(checks: Vec<VoiceDoctorCheck>) -> Vec<VoiceDoctorCheck> {
    checks
        .into_iter()
        .filter_map(|check| {
            let name = check.name.trim();
            if name.is_empty() {
                return None;
            }
            Some(VoiceDoctorCheck {
                name: redact(name),
                status: normalize_voice_check_status(&check.status),
                message: redact(check.message.trim()),
            })
        })
        .collect()
}

pub(crate) fn normalize_voice_check_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "ok" | "pass" | "passed" | "ready" => "ok",
        "warn" | "warning" | "degraded" | "unknown" => "warn",
        "error" | "fail" | "failed" | "blocked" => "error",
        _ => "warn",
    }
    .to_owned()
}

pub(crate) fn voice_check_summary_status(checks: &[VoiceDoctorCheck]) -> String {
    if checks.iter().any(|check| check.status == "error") {
        "error".to_owned()
    } else if checks.is_empty() || checks.iter().any(|check| check.status == "warn") {
        "warn".to_owned()
    } else {
        "ok".to_owned()
    }
}

pub(crate) fn voice_checks_summary(checks: &[VoiceDoctorCheck]) -> String {
    let errors = checks
        .iter()
        .filter(|check| check.status == "error")
        .count();
    let warnings = checks.iter().filter(|check| check.status == "warn").count();
    if errors > 0 {
        format!("{errors} blocking voice issue{}", plural(errors))
    } else if warnings > 0 {
        format!("{warnings} voice warning{}", plural(warnings))
    } else if checks.is_empty() {
        "no bridge checks reported".to_owned()
    } else {
        "voice bridge ready".to_owned()
    }
}

pub(crate) fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

pub(crate) fn voice_runtime_state(checks: &[VoiceDoctorCheck]) -> VoiceRuntimeState {
    if checks.iter().any(|check| check.status == "error") {
        VoiceRuntimeState::Blocked
    } else if checks.iter().any(|check| check.status == "warn") {
        VoiceRuntimeState::Degraded
    } else {
        VoiceRuntimeState::Ready
    }
}

pub(crate) fn normalize_voice_config_string(value: &str) -> String {
    value.trim().to_owned()
}

pub(crate) fn is_supported_voice_provider(provider: &str) -> bool {
    matches!(provider, "edge" | "openai" | "system" | "stub")
}

pub(crate) fn clamp_voice_adjustment(value: i16) -> i16 {
    value.clamp(-50, 50)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_tts_maps_configured_voice_ids_to_default() {
        for voice_id in [
            "",
            "default",
            "id-ID-GadisNeural",
            "en-US-AvaNeural",
            "bad; Start-Process calc",
        ] {
            assert_eq!(system_tts_voice_id(voice_id), SYSTEM_DEFAULT_VOICE_ID);
        }
    }

    #[test]
    fn system_tts_macos_plan_uses_default_voice_without_voice_flag() {
        let plan = system_tts_command_plan(
            SystemTtsPlatform::Macos,
            "say",
            "Hello",
            "id-ID-GadisNeural",
            0,
        );

        assert_eq!(plan.binary, "say");
        assert_eq!(plan.voice_id, SYSTEM_DEFAULT_VOICE_ID);
        assert_eq!(plan.stdin_text, None);
        assert_eq!(
            plan.args,
            vec!["-r".to_owned(), "175".to_owned(), "Hello".to_owned()]
        );
        assert!(!plan
            .args
            .iter()
            .any(|arg| arg == "-v" || arg == "id-ID-GadisNeural"));
    }

    #[test]
    fn system_tts_linux_plan_uses_default_voice_without_voice_flag() {
        let plan = system_tts_command_plan(
            SystemTtsPlatform::Linux,
            "espeak",
            "Hello",
            "id-ID-GadisNeural",
            0,
        );

        assert_eq!(plan.binary, "espeak");
        assert_eq!(plan.voice_id, SYSTEM_DEFAULT_VOICE_ID);
        assert_eq!(plan.stdin_text, None);
        assert_eq!(
            plan.args,
            vec!["-s".to_owned(), "175".to_owned(), "Hello".to_owned()]
        );
        assert!(!plan
            .args
            .iter()
            .any(|arg| arg == "-v" || arg == "id-ID-GadisNeural"));
    }

    #[test]
    fn system_tts_windows_plan_passes_speech_text_on_stdin() {
        let text = "hello\"; Start-Process calc; #";
        let plan = system_tts_command_plan(
            SystemTtsPlatform::Windows,
            "powershell",
            text,
            "id-ID-GadisNeural",
            0,
        );

        assert_eq!(plan.binary, "powershell");
        assert_eq!(plan.voice_id, SYSTEM_DEFAULT_VOICE_ID);
        assert_eq!(plan.stdin_text.as_deref(), Some(text));
        assert!(plan.args.iter().any(|arg| arg == "-EncodedCommand"));
        assert!(!plan
            .args
            .iter()
            .any(|arg| arg.contains(text) || arg.contains("Start-Process")));
    }

    #[test]
    fn powershell_script_is_encoded_as_utf16le_base64() {
        assert_eq!(base64_encode_utf16le("A"), "QQA=");
        let script = system_tts_powershell_script(50);
        assert!(script.contains("[Console]::In.ReadToEnd()"));
        assert!(script.contains("$synth.Rate = 5"));
        assert!(!script.contains("Start-Process"));
        assert_eq!(ps_rate(500), 10);
        assert_eq!(ps_rate(-500), -10);
    }

    #[test]
    fn system_tts_path_lookup_checks_presence_without_version_flags() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(temp.path().join("say"), b"").expect("say fixture should write");
        std::fs::write(temp.path().join("powershell.EXE"), b"")
            .expect("powershell fixture should write");
        let path_value =
            std::env::join_paths(std::iter::once(temp.path())).expect("path should join");

        assert!(command_exists_in_path(
            "say",
            SystemTtsPlatform::Macos,
            &path_value,
            None,
        ));
        assert!(command_exists_in_path(
            "powershell",
            SystemTtsPlatform::Windows,
            &path_value,
            Some(std::ffi::OsStr::new(".EXE;.CMD")),
        ));
        assert!(!command_exists_in_path(
            "espeak",
            SystemTtsPlatform::Linux,
            &path_value,
            None,
        ));
    }

    #[test]
    fn openai_tts_curl_args_fail_on_http_errors_and_write_status() {
        let audio_path = Path::new("/tmp/cadis-openai-test.mp3");
        let args = openai_tts_curl_args(
            "https://example.test/v1/audio/speech",
            r#"{"input":"hello"}"#,
            audio_path,
            "literal-test-key",
        );

        assert!(args.contains(&"--fail-with-body".to_owned()));
        assert!(args.contains(&"--show-error".to_owned()));
        assert!(args.contains(&"--write-out".to_owned()));
        assert!(args.contains(&"%{http_code}".to_owned()));
        assert!(args.contains(&"--output".to_owned()));
        assert!(args.contains(&audio_path.to_string_lossy().into_owned()));
    }

    #[test]
    fn openai_tts_parses_curl_http_status() {
        assert_eq!(parse_curl_http_status(b"200"), Some(200));
        assert_eq!(parse_curl_http_status(b"401"), Some(401));
        assert_eq!(parse_curl_http_status(b"000"), None);
        assert_eq!(parse_curl_http_status(b"not-a-status"), None);
    }

    #[test]
    fn openai_tts_audio_validation_accepts_mp3_signatures() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let id3_path = temp_dir.path().join("id3.mp3");
        let frame_path = temp_dir.path().join("frame.mp3");

        let mut id3_mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x21".to_vec();
        id3_mp3.extend([0_u8; 32]);
        fs::write(&id3_path, &id3_mp3).expect("ID3 fixture should be written");

        let mut frame_mp3 = vec![0xff, 0xfb, 0x90, 0x64];
        frame_mp3.extend([0_u8; 32]);
        fs::write(&frame_path, &frame_mp3).expect("frame fixture should be written");

        assert_eq!(
            validate_openai_tts_audio_file(&id3_path, "literal-test-key").unwrap(),
            id3_mp3.len() as u64
        );
        assert_eq!(
            validate_openai_tts_audio_file(&frame_path, "literal-test-key").unwrap(),
            frame_mp3.len() as u64
        );
    }

    #[test]
    fn openai_tts_audio_validation_rejects_json_error_body() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let body_path = temp_dir.path().join("error.mp3");
        let api_key = "literal-test-key";
        fs::write(
            &body_path,
            format!(r#"{{"error":{{"message":"invalid API key {api_key}"}}}}"#),
        )
        .expect("error fixture should be written");

        let error = validate_openai_tts_audio_file(&body_path, api_key).unwrap_err();

        assert_eq!(error.code, "openai_tts_invalid_audio");
        assert!(error.message.contains("not a plausible MP3"));
        assert!(error.message.contains("response body"));
        assert!(error.message.contains("[REDACTED]"));
        assert!(!error.message.contains(api_key));
    }

    #[test]
    fn openai_tts_failure_message_redacts_stderr_and_body() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let body_path = temp_dir.path().join("error.mp3");
        let api_key = "literal-test-key";
        fs::write(
            &body_path,
            format!(r#"{{"error":{{"message":"Authorization: Bearer {api_key}"}}}}"#),
        )
        .expect("error fixture should be written");

        let message = openai_tts_failure_message(
            Some(401),
            Some(22),
            format!("curl: (22) failed for {api_key}").as_bytes(),
            &body_path,
            api_key,
        );

        assert!(message.contains("HTTP 401"));
        assert!(message.contains("curl exit 22"));
        assert!(message.contains("curl stderr"));
        assert!(message.contains("response body"));
        assert!(message.contains("[REDACTED]"));
        assert!(!message.contains(api_key));
    }
}
