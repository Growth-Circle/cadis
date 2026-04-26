//! Local CADIS configuration, state layout, redaction, and JSONL logs.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use cadis_protocol::{EventEnvelope, SessionId};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Runtime model provider configuration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ModelConfig {
    /// Provider label. Supported values for the desktop MVP are `auto`, `codex-cli`, `echo`, `ollama`, and `openai`.
    pub provider: String,
    /// Ollama model name.
    pub ollama_model: String,
    /// Ollama HTTP endpoint.
    pub ollama_endpoint: String,
    /// OpenAI model name.
    pub openai_model: String,
    /// OpenAI API base URL.
    pub openai_base_url: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_owned(),
            ollama_model: "llama3.2".to_owned(),
            ollama_endpoint: "http://127.0.0.1:11434".to_owned(),
            openai_model: "gpt-5.2".to_owned(),
            openai_base_url: "https://api.openai.com/v1".to_owned(),
        }
    }
}

/// HUD preference configuration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct HudConfig {
    /// HUD theme key.
    pub theme: String,
    /// Center avatar style key.
    pub avatar_style: String,
    /// Background opacity percentage.
    pub background_opacity: u8,
    /// Always-on-top preference.
    pub always_on_top: bool,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            theme: "arc".to_owned(),
            avatar_style: "orb".to_owned(),
            background_opacity: 90,
            always_on_top: false,
        }
    }
}

/// Voice preference configuration used by the HUD prototype.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Whether voice output is enabled.
    pub enabled: bool,
    /// Voice identifier.
    pub voice_id: String,
    /// Speaking rate adjustment.
    pub rate: i16,
    /// Pitch adjustment.
    pub pitch: i16,
    /// Volume adjustment.
    pub volume: i16,
    /// Whether completed assistant messages should be spoken.
    pub auto_speak: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            voice_id: "id-ID-GadisNeural".to_owned(),
            rate: 0,
            pitch: 0,
            volume: 0,
            auto_speak: false,
        }
    }
}

/// CADIS daemon configuration loaded from env and `config.toml`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct CadisConfig {
    /// Local CADIS state directory.
    pub cadis_home: PathBuf,
    /// Log level string used by launchers.
    pub log_level: String,
    /// Optional socket path override.
    pub socket_path: Option<PathBuf>,
    /// Model provider settings.
    pub model: ModelConfig,
    /// HUD settings.
    pub hud: HudConfig,
    /// Voice settings.
    pub voice: VoiceConfig,
}

impl Default for CadisConfig {
    fn default() -> Self {
        Self {
            cadis_home: default_cadis_home(),
            log_level: "info".to_owned(),
            socket_path: None,
            model: ModelConfig::default(),
            hud: HudConfig::default(),
            voice: VoiceConfig::default(),
        }
    }
}

impl CadisConfig {
    /// Returns the loaded configuration file path.
    pub fn config_path(&self) -> PathBuf {
        self.cadis_home.join("config.toml")
    }

    /// Returns the socket path used by the daemon.
    pub fn effective_socket_path(&self) -> PathBuf {
        if let Some(path) = &self.socket_path {
            return expand_home(path);
        }

        default_socket_path(&self.cadis_home)
    }

    /// Returns daemon-owned UI preferences as JSON.
    pub fn ui_preferences(&self) -> serde_json::Value {
        serde_json::json!({
            "hud": {
                "theme": self.hud.theme,
                "avatar_style": self.hud.avatar_style,
                "background_opacity": self.hud.background_opacity,
                "always_on_top": self.hud.always_on_top
            },
            "voice": {
                "enabled": self.voice.enabled,
                "voice_id": self.voice.voice_id,
                "rate": self.voice.rate,
                "pitch": self.voice.pitch,
                "volume": self.voice.volume,
                "auto_speak": self.voice.auto_speak
            }
        })
    }
}

/// Errors emitted by local store helpers.
#[derive(Debug)]
pub enum StoreError {
    /// File-system I/O failed.
    Io(std::io::Error),
    /// Configuration TOML failed to parse.
    Toml(toml::de::Error),
    /// Event failed to serialize.
    Json(serde_json::Error),
    /// Home directory could not be discovered.
    MissingHome,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "store I/O failed: {error}"),
            Self::Toml(error) => write!(formatter, "config TOML is invalid: {error}"),
            Self::Json(error) => write!(formatter, "event JSON serialization failed: {error}"),
            Self::MissingHome => formatter.write_str("HOME is not set"),
        }
    }
}

impl Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<toml::de::Error> for StoreError {
    fn from(error: toml::de::Error) -> Self {
        Self::Toml(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Loads CADIS config from env defaults plus `config.toml` when present.
pub fn load_config() -> Result<CadisConfig, StoreError> {
    let mut config = CadisConfig::default();
    ensure_layout(&config)?;

    let config_path = config.config_path();
    if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        let mut file_config = toml::from_str::<CadisConfig>(&content)?;
        if file_config.cadis_home.as_os_str().is_empty() {
            file_config.cadis_home = config.cadis_home;
        } else {
            file_config.cadis_home = expand_home(&file_config.cadis_home);
        }
        config = file_config;
    }

    if let Ok(level) = env::var("CADIS_LOG_LEVEL") {
        if !level.trim().is_empty() {
            config.log_level = level;
        }
    }

    if let Ok(provider) = env::var("CADIS_MODEL_PROVIDER") {
        if !provider.trim().is_empty() {
            config.model.provider = provider;
        }
    }

    ensure_layout(&config)?;
    Ok(config)
}

/// Returns the OpenAI API key from CADIS-supported environment variables.
pub fn openai_api_key_from_env() -> Option<String> {
    openai_api_key_from_lookup(|name| env::var(name).ok())
}

/// Creates the local CADIS state layout.
pub fn ensure_layout(config: &CadisConfig) -> Result<(), StoreError> {
    create_private_dir(&config.cadis_home)?;

    for path in [
        "logs",
        "sessions",
        "workers",
        "worktrees",
        "run",
        "tokens",
        "approvals",
    ] {
        create_private_dir(&config.cadis_home.join(path))?;
    }

    Ok(())
}

/// Returns the default CADIS home directory.
pub fn default_cadis_home() -> PathBuf {
    env::var_os("CADIS_HOME")
        .map(PathBuf::from)
        .map(|path| expand_home(&path))
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".cadis")
        })
}

/// Returns the default local daemon socket path.
pub fn default_socket_path(cadis_home: &Path) -> PathBuf {
    if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.trim().is_empty() {
            return PathBuf::from(runtime_dir).join("cadis").join("cadisd.sock");
        }
    }

    cadis_home.join("run").join("cadisd.sock")
}

/// Replaces secret-looking values with `[REDACTED]`.
pub fn redact(input: &str) -> String {
    let mut output = input.to_owned();
    for regex in prefix_redaction_patterns() {
        output = regex.replace_all(&output, "$1[REDACTED]").into_owned();
    }
    for regex in value_redaction_patterns() {
        output = regex.replace_all(&output, "[REDACTED]").into_owned();
    }
    output
}

/// Append-only JSONL event log writer.
#[derive(Clone, Debug)]
pub struct EventLog {
    logs_dir: PathBuf,
}

impl EventLog {
    /// Creates an event log rooted under the configured CADIS home.
    pub fn new(config: &CadisConfig) -> Self {
        Self {
            logs_dir: config.cadis_home.join("logs"),
        }
    }

    /// Appends one redacted event envelope to the appropriate JSONL log.
    pub fn append_event(&self, event: &EventEnvelope) -> Result<(), StoreError> {
        create_private_dir(&self.logs_dir)?;
        let json = serde_json::to_string(event)?;
        let mut redacted = redact(&json);
        redacted.push('\n');
        let path = self.event_path(event.session_id.as_ref());
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(redacted.as_bytes())?;
        Ok(())
    }

    fn event_path(&self, session_id: Option<&SessionId>) -> PathBuf {
        let name = session_id
            .map(|session_id| safe_file_component(session_id.as_str()))
            .unwrap_or_else(|| "daemon".to_owned());
        self.logs_dir.join(format!("{name}.jsonl"))
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let Some(value) = path.to_str() else {
        return path.to_path_buf();
    };

    if value == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }

    if let Some(rest) = value.strip_prefix("~/") {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(value));
    }

    path.to_path_buf()
}

fn create_private_dir(path: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(path)?;
    set_private_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

fn prefix_redaction_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r#"(?i)\b((?:authorization)\s*[:=]\s*["']?(?:bearer\s+)?)[^"',\s}]+"#,
            r#"(?i)\b((?:cadis_openai|openai|codex|anthropic|gemini|openrouter)?_?api[_-]?key\s*[:=]\s*["']?)[^"',\s}]+"#,
            r#"(?i)\b((?:telegram_)?bot_token\s*[:=]\s*["']?)[^"',\s}]+"#,
            r#"(?i)\b((?:token|secret)\s*[:=]\s*["']?)[^"',\s}]+"#,
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("redaction regex should compile"))
        .collect()
    })
}

fn value_redaction_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r#"\b(sk-[A-Za-z0-9_-]{16,})\b"#,
            r#"\b(AIza[0-9A-Za-z_-]{20,})\b"#,
            r#"\b([0-9]{8,10}:[A-Za-z0-9_-]{35,})\b"#,
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("redaction regex should compile"))
        .collect()
    })
}

fn safe_file_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn openai_api_key_from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Option<String> {
    ["CADIS_OPENAI_API_KEY", "OPENAI_API_KEY"]
        .into_iter()
        .find_map(|name| lookup(name).filter(|value| !value.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_provider_keys_and_tokens() {
        let input = "OPENAI_API_KEY=sk-testsecretvalue123456 TELEGRAM_BOT_TOKEN=123456789:abcdefghijklmnopqrstuvwxyzABCDEFGHI";
        let output = redact(input);

        assert!(output.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(output.contains("TELEGRAM_BOT_TOKEN=[REDACTED]"));
        assert!(!output.contains("sk-testsecretvalue123456"));
        assert!(!output.contains("abcdefghijklmnopqrstuvwxyzABCDEFGHI"));
    }

    #[test]
    fn redacts_cadis_openai_key_and_bearer_auth() {
        let input =
            "CADIS_OPENAI_API_KEY=test-secret CODEX_API_KEY=codex-secret Authorization: Bearer sk-testsecretvalue123456";
        let output = redact(input);

        assert!(output.contains("CADIS_OPENAI_API_KEY=[REDACTED]"));
        assert!(output.contains("CODEX_API_KEY=[REDACTED]"));
        assert!(output.contains("Authorization: Bearer [REDACTED]"));
        assert!(!output.contains("test-secret"));
        assert!(!output.contains("codex-secret"));
        assert!(!output.contains("sk-testsecretvalue123456"));
    }

    #[test]
    fn parses_openai_model_config_without_key_field() {
        let config = toml::from_str::<CadisConfig>(
            r#"
            [model]
            provider = "openai"
            openai_model = "gpt-5.2"
            openai_base_url = "https://api.openai.com/v1"
            "#,
        )
        .expect("OpenAI config should parse");

        assert_eq!(config.model.provider, "openai");
        assert_eq!(config.model.openai_model, "gpt-5.2");
        assert_eq!(config.model.openai_base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn openai_api_key_helper_uses_supported_env_names_only() {
        let key = openai_api_key_from_lookup(|name| match name {
            "CADIS_OPENAI_API_KEY" => Some("cadis-key".to_owned()),
            "OPENAI_API_KEY" => Some("global-key".to_owned()),
            _ => Some("unsupported-key".to_owned()),
        });

        assert_eq!(key.as_deref(), Some("cadis-key"));

        let key = openai_api_key_from_lookup(|name| match name {
            "OPENAI_API_KEY" => Some("global-key".to_owned()),
            _ => None,
        });

        assert_eq!(key.as_deref(), Some("global-key"));

        let key = openai_api_key_from_lookup(|name| match name {
            "OPENAI_API_KEY" => Some("  ".to_owned()),
            _ => None,
        });

        assert_eq!(key, None);
    }

    #[test]
    fn safe_component_replaces_path_separators() {
        assert_eq!(safe_file_component("ses/../1"), "ses____1");
    }
}
