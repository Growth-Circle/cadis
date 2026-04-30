//! Voice, TTS providers, speech decisions, and voice doctor utilities.

use super::*;

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

        pub(crate) const MAX_SYSTEM_TTS_CHARS: usize = 2000;
        let text = if text.chars().count() > MAX_SYSTEM_TTS_CHARS {
            truncate_to_utf8_boundary(text, MAX_SYSTEM_TTS_CHARS).0
        } else {
            text
        };

        let result = if cfg!(target_os = "macos") {
            Command::new("say")
                .arg("-v")
                .arg(request.voice_id)
                .arg("--rate")
                .arg(format!("{}", words_per_minute(request.rate)))
                .arg(text)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
        } else if cfg!(target_os = "windows") {
            let ps_script = format!(
                "Add-Type -AssemblyName System.Speech; \
                 $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
                 $synth.Rate = {}; \
                 $synth.Speak(\"{}\")",
                ps_rate(request.rate),
                text.replace('\\', "\\\\").replace('"', "\\\"")
            );
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps_script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
        } else {
            let speed_arg = format!("{}", words_per_minute(request.rate));
            Command::new("espeak")
                .args(["-v", request.voice_id, "-s", &speed_arg])
                .arg(text)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
        };

        match result {
            Ok(output) if output.status.success() => Ok(TtsOutput {
                provider: "system".to_owned(),
                voice_id: request.voice_id.to_owned(),
                spoken_chars: request.text.chars().count(),
                audio_path: None,
            }),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(TtsError::new(
                    "system_tts_failed",
                    format!("system TTS failed: {}", stderr.trim()),
                    true,
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let binary = if cfg!(target_os = "macos") {
                    "say"
                } else if cfg!(target_os = "windows") {
                    "powershell"
                } else {
                    "espeak"
                };
                Err(TtsError::new(
                    "system_tts_not_found",
                    format!("{binary} is not installed; system TTS requires {binary}"),
                    false,
                ))
            }
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

fn words_per_minute(rate: i16) -> u32 {
    let base: f64 = 175.0;
    let adjusted = base * (1.0 + f64::from(rate) / 100.0);
    adjusted.clamp(80.0, 450.0) as u32
}

fn ps_rate(rate: i16) -> i32 {
    (f64::from(rate) / 10.0).round() as i32
}

fn system_tts_available() -> bool {
    let binary = if cfg!(target_os = "macos") {
        "say"
    } else if cfg!(target_os = "windows") {
        "powershell"
    } else {
        "espeak"
    };
    Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub(crate) fn tts_provider_from_config(provider: &str) -> Box<dyn TtsProvider> {
    match provider {
        "edge" => Box::new(EdgeTtsProvider),
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
