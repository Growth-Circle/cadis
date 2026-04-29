//! Local CADIS configuration, state layout, redaction, durable state, and JSONL logs.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use cadis_protocol::{
    AgentId, AgentSessionId, ApprovalDecision, ApprovalId, EventEnvelope, RiskClass, SessionId,
    Timestamp, ToolCallId,
};
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Runtime model provider configuration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ModelConfig {
    /// Default provider label. Per-agent selections may use `provider/model`.
    /// Supported provider values for the desktop MVP are `auto`, `codex-cli`, `echo`, `ollama`, and `openai`.
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
    /// TTS provider identifier.
    pub provider: String,
    /// Voice identifier.
    pub voice_id: String,
    /// STT language, usually `auto` or an ISO 639-1 language code.
    pub stt_language: String,
    /// Speaking rate adjustment.
    pub rate: i16,
    /// Pitch adjustment.
    pub pitch: i16,
    /// Volume adjustment.
    pub volume: i16,
    /// Whether completed assistant messages should be spoken.
    pub auto_speak: bool,
    /// Maximum response length eligible for direct speech.
    pub max_spoken_chars: usize,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "edge".to_owned(),
            voice_id: "id-ID-GadisNeural".to_owned(),
            stt_language: "auto".to_owned(),
            rate: 0,
            pitch: 0,
            volume: 0,
            auto_speak: false,
            max_spoken_chars: 800,
        }
    }
}

/// Daemon limits for request-driven `agent.spawn`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentSpawnConfig {
    /// Maximum child depth below a root agent.
    pub max_depth: usize,
    /// Maximum direct children any one parent may own.
    pub max_children_per_parent: usize,
    /// Maximum total registered agents, including built-in agents.
    pub max_total_agents: usize,
}

impl Default for AgentSpawnConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_children_per_parent: 4,
            max_total_agents: 32,
        }
    }
}

/// Daemon-owned Agent Runtime settings.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentRuntimeConfig {
    /// Default per-route AgentSession timeout.
    pub default_timeout_sec: i64,
    /// Maximum state-machine steps per AgentSession.
    pub max_steps_per_session: u32,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            default_timeout_sec: 900,
            max_steps_per_session: 8,
        }
    }
}

/// Daemon-owned orchestration settings.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct OrchestratorConfig {
    /// Enables explicit `/worker`, `/spawn`, `/route`, and `/delegate` message actions.
    pub worker_delegation_enabled: bool,
    /// Role used by `/worker` when the message does not include `Role: task`.
    pub default_worker_role: String,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            worker_delegation_enabled: true,
            default_worker_role: "Worker".to_owned(),
        }
    }
}

/// Profile selection for daemon-owned profile state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ProfileConfig {
    /// Default profile ID under `~/.cadis/profiles/<profile>`.
    pub default_profile: String,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            default_profile: "default".to_owned(),
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
    /// Optional TCP port for cross-platform transport (default: None, Unix socket).
    pub tcp_port: Option<u16>,
    /// Model provider settings.
    pub model: ModelConfig,
    /// HUD settings.
    pub hud: HudConfig,
    /// Voice settings.
    pub voice: VoiceConfig,
    /// Request-driven agent spawn limits.
    pub agent_spawn: AgentSpawnConfig,
    /// Per-route AgentSession runtime limits.
    pub agent_runtime: AgentRuntimeConfig,
    /// Daemon-owned orchestrator settings.
    pub orchestrator: OrchestratorConfig,
    /// Profile-home selection and profile layout settings.
    pub profile: ProfileConfig,
    /// Policy engine settings.
    pub policy: cadis_policy::PolicyConfig,
}

impl Default for CadisConfig {
    fn default() -> Self {
        Self {
            cadis_home: default_cadis_home(),
            log_level: "info".to_owned(),
            socket_path: None,
            tcp_port: None,
            model: ModelConfig::default(),
            hud: HudConfig::default(),
            voice: VoiceConfig::default(),
            agent_spawn: AgentSpawnConfig::default(),
            agent_runtime: AgentRuntimeConfig::default(),
            orchestrator: OrchestratorConfig::default(),
            profile: ProfileConfig::default(),
            policy: cadis_policy::PolicyConfig::default(),
        }
    }
}

impl CadisConfig {
    /// Returns the loaded configuration file path.
    pub fn config_path(&self) -> PathBuf {
        self.cadis_home.join("config.toml")
    }

    /// Returns the socket path used by the daemon.
    ///
    /// On Windows this returns a conventional path, but the daemon should use
    /// TCP transport instead of Unix sockets. Callers should check
    /// [`default_socket_path`] for `None` to detect TCP-only platforms.
    pub fn effective_socket_path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.socket_path {
            return Some(expand_home(path));
        }

        default_socket_path(&self.cadis_home)
    }

    /// Returns the TCP address for cross-platform transport.
    ///
    /// Uses the configured `tcp_port` or the default `7433`.
    pub fn effective_tcp_address(&self) -> String {
        let port = self.tcp_port.unwrap_or(7433);
        format!("127.0.0.1:{port}")
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
                "provider": self.voice.provider,
                "voice_id": self.voice.voice_id,
                "stt_language": self.voice.stt_language,
                "rate": self.voice.rate,
                "pitch": self.voice.pitch,
                "volume": self.voice.volume,
                "auto_speak": self.voice.auto_speak,
                "max_spoken_chars": self.voice.max_spoken_chars
            },
            "agent_spawn": {
                "max_depth": self.agent_spawn.max_depth,
                "max_children_per_parent": self.agent_spawn.max_children_per_parent,
                "max_total_agents": self.agent_spawn.max_total_agents
            },
            "agent_runtime": {
                "default_timeout_sec": self.agent_runtime.default_timeout_sec,
                "max_steps_per_session": self.agent_runtime.max_steps_per_session
            },
            "orchestrator": {
                "worker_delegation_enabled": self.orchestrator.worker_delegation_enabled,
                "default_worker_role": self.orchestrator.default_worker_role
            },
            "policy": serde_json::to_value(&self.policy).unwrap_or_default()
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
    /// JSON failed to serialize or parse.
    Json(serde_json::Error),
    /// TOML serialization failed.
    TomlSerialize(toml::ser::Error),
    /// Home directory could not be discovered.
    MissingHome,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "store I/O failed: {error}"),
            Self::Toml(error) => write!(formatter, "config TOML is invalid: {error}"),
            Self::Json(error) => write!(formatter, "store JSON failed: {error}"),
            Self::TomlSerialize(error) => write!(formatter, "store TOML failed: {error}"),
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

impl From<toml::ser::Error> for StoreError {
    fn from(error: toml::ser::Error) -> Self {
        Self::TomlSerialize(error)
    }
}

/// CADIS home resolver rooted at `~/.cadis` by default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CadisHome {
    root: PathBuf,
}

impl CadisHome {
    /// Creates a resolver for an explicit CADIS home path.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: expand_home(root.as_ref()),
        }
    }

    /// Creates a resolver for the environment/default CADIS home path.
    pub fn default_home() -> Self {
        Self::new(default_cadis_home())
    }

    /// Returns the CADIS home root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the global config path.
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// Returns a profile-home resolver for `profile_id`.
    pub fn profile(&self, profile_id: impl AsRef<str>) -> ProfileHome {
        ProfileHome::new(self.root.clone(), profile_id)
    }

    /// Initializes the top-level CADIS home and one profile home.
    pub fn init_profile(&self, profile_id: impl AsRef<str>) -> Result<ProfileHome, StoreError> {
        create_private_dir(&self.root)?;
        for path in ["profiles", "global-cache", "plugins", "bin", "logs", "run"] {
            create_private_dir(&self.root.join(path))?;
        }

        let profile = self.profile(profile_id);
        profile.init_template()?;
        Ok(profile)
    }
}

impl Default for CadisHome {
    fn default() -> Self {
        Self::default_home()
    }
}

/// Profile home resolver rooted under `~/.cadis/profiles/<profile>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileHome {
    profile_id: String,
    root: PathBuf,
}

impl ProfileHome {
    /// Creates a profile resolver under an explicit CADIS home root.
    pub fn new(cadis_home: impl AsRef<Path>, profile_id: impl AsRef<str>) -> Self {
        let profile_id = safe_file_stem(profile_id.as_ref());
        Self {
            root: cadis_home.as_ref().join("profiles").join(&profile_id),
            profile_id,
        }
    }

    /// Returns the profile ID.
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// Returns the profile home root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the profile config path.
    pub fn profile_config_path(&self) -> PathBuf {
        self.root.join("profile.toml")
    }

    /// Returns the profile `.gitignore` path.
    pub fn gitignore_path(&self) -> PathBuf {
        self.root.join(".gitignore")
    }

    /// Returns a persistent agent home path.
    pub fn agent_home(&self, agent_id: impl AsRef<str>) -> PathBuf {
        self.root
            .join("agents")
            .join(safe_file_stem(agent_id.as_ref()))
    }

    /// Returns a persistent agent-home resolver.
    pub fn agent(&self, agent_id: impl AsRef<str>) -> AgentHome {
        AgentHome::new(self.clone(), agent_id)
    }

    /// Returns the profile workspace metadata directory.
    pub fn workspaces_dir(&self) -> PathBuf {
        self.root.join("workspaces")
    }

    /// Returns the profile workspace registry TOML path.
    pub fn workspace_registry_path(&self) -> PathBuf {
        self.workspaces_dir().join("registry.toml")
    }

    /// Returns the profile workspace aliases TOML path.
    pub fn workspace_aliases_path(&self) -> PathBuf {
        self.workspaces_dir().join("aliases.toml")
    }

    /// Returns the profile workspace grant JSONL path.
    pub fn workspace_grants_path(&self) -> PathBuf {
        self.workspaces_dir().join("grants.jsonl")
    }

    /// Returns the profile worker artifact root.
    pub fn worker_artifacts_dir(&self) -> PathBuf {
        self.root.join("artifacts").join("workers")
    }

    /// Returns conventional artifact paths for one worker.
    pub fn worker_artifact_paths(&self, worker_id: impl AsRef<str>) -> WorkerArtifactPathSet {
        WorkerArtifactPathSet::new(self.worker_artifacts_dir(), worker_id)
    }

    /// Ensures the artifact root for one worker exists.
    pub fn ensure_worker_artifact_layout(
        &self,
        worker_id: impl AsRef<str>,
    ) -> Result<WorkerArtifactPathSet, StoreError> {
        let paths = self.worker_artifact_paths(worker_id);
        create_private_dir(&paths.root)?;
        Ok(paths)
    }

    /// Ensures the profile directory skeleton exists with private permissions.
    pub fn ensure_layout(&self) -> Result<(), StoreError> {
        create_private_dir(&self.root)?;
        for path in [
            "secrets",
            "channels",
            "agents",
            "memory/global",
            "memory/daily",
            "memory/projects",
            "memory/delegation",
            "memory/vector",
            "memory/candidates",
            "skills/candidates",
            "skills/approved",
            "skills/archived",
            "workspaces",
            "workers",
            "sessions/agent",
            "sessions/worker",
            "sessions/channel",
            "artifacts/workers",
            "checkpoints",
            "sandboxes",
            "eventlog",
            "cron",
            "logs",
            "locks",
            "run",
        ] {
            create_private_dir(&self.root.join(path))?;
        }

        Ok(())
    }

    /// Initializes profile template files without overwriting user edits.
    pub fn init_template(&self) -> Result<(), StoreError> {
        self.ensure_layout()?;
        write_template_file_if_missing(&self.gitignore_path(), profile_gitignore_template())?;
        write_template_file_if_missing(
            &self.profile_config_path(),
            &profile_toml_template(&self.profile_id),
        )?;
        write_template_file_if_missing(&self.workspace_registry_path(), "workspace = []\n")?;
        write_template_file_if_missing(&self.workspace_aliases_path(), "alias = []\n")?;
        Ok(())
    }

    /// Initializes one persistent agent home without overwriting user edits.
    pub fn init_agent(&self, template: &AgentHomeTemplate) -> Result<AgentHome, StoreError> {
        let agent = self.agent(template.agent_id.as_str());
        agent.init_template(template)?;
        Ok(agent)
    }

    /// Runs lightweight profile and agent-home diagnostics.
    pub fn agent_doctor_diagnostics(
        &self,
        options: AgentHomeDoctorOptions,
    ) -> Result<Vec<AgentHomeDiagnostic>, StoreError> {
        self.ensure_layout()?;

        let agents_dir = self.root.join("agents");
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        for entry in fs::read_dir(&agents_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            count += 1;
            let agent_id = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown");
            diagnostics.extend(AgentHome::from_root(agent_id.to_owned(), path).doctor(&options));
        }

        diagnostics.push(AgentHomeDiagnostic {
            name: "profile.agents".to_owned(),
            status: if count == 0 { "warn" } else { "ok" }.to_owned(),
            message: format!("{count} agent home(s) found"),
        });

        Ok(diagnostics)
    }

    /// Creates a workspace registry helper for this profile.
    pub fn workspace_registry(&self) -> WorkspaceRegistryStore {
        WorkspaceRegistryStore::new(self.clone())
    }

    /// Creates a workspace grant helper for this profile.
    pub fn workspace_grants(&self) -> WorkspaceGrantStore {
        WorkspaceGrantStore::new(self.clone())
    }
}

/// Persistent agent-home resolver rooted under a profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentHome {
    agent_id: String,
    root: PathBuf,
}

impl AgentHome {
    /// Creates an agent-home resolver for a profile.
    pub fn new(profile_home: ProfileHome, agent_id: impl AsRef<str>) -> Self {
        let agent_id = safe_file_stem(agent_id.as_ref());
        Self {
            root: profile_home.agent_home(&agent_id),
            agent_id,
        }
    }

    fn from_root(agent_id: String, root: PathBuf) -> Self {
        Self { agent_id, root }
    }

    /// Returns the safe profile-local agent ID component.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Returns the agent-home root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the path to `AGENT.toml`.
    pub fn agent_toml_path(&self) -> PathBuf {
        self.root.join("AGENT.toml")
    }

    /// Returns the path to `POLICY.toml`.
    pub fn policy_toml_path(&self) -> PathBuf {
        self.root.join("POLICY.toml")
    }

    /// Ensures the agent-home directory skeleton exists with private permissions.
    pub fn ensure_layout(&self) -> Result<(), StoreError> {
        create_private_dir(&self.root)?;
        for path in ["skills", "memory", "memory/daily", "prompts", "sessions"] {
            create_private_dir(&self.root.join(path))?;
        }
        Ok(())
    }

    /// Initializes agent templates without overwriting user edits.
    pub fn init_template(&self, template: &AgentHomeTemplate) -> Result<(), StoreError> {
        self.ensure_layout()?;
        write_template_file_if_missing(&self.agent_toml_path(), &agent_toml_template(template)?)?;
        write_template_file_if_missing(&self.root.join("PERSONA.md"), &persona_template(template))?;
        write_template_file_if_missing(
            &self.root.join("INSTRUCTIONS.md"),
            &instructions_template(template),
        )?;
        write_template_file_if_missing(&self.root.join("USER.md"), user_template())?;
        write_template_file_if_missing(&self.root.join("MEMORY.md"), memory_template())?;
        write_template_file_if_missing(&self.root.join("TOOLS.md"), tools_template())?;
        write_template_file_if_missing(&self.policy_toml_path(), &agent_policy_toml_template()?)?;
        write_template_file_if_missing(
            &self.root.join("SKILL_POLICY.toml"),
            &skill_policy_toml_template()?,
        )?;
        write_template_file_if_missing(&self.root.join("README.md"), agent_readme_template())?;
        write_template_file_if_missing(&self.root.join("memory/decisions.md"), "# Decisions\n")?;
        write_template_file_if_missing(&self.root.join("memory/delegation.md"), "# Delegation\n")?;
        Ok(())
    }

    /// Loads typed `AGENT.toml` metadata.
    pub fn load_metadata(&self) -> Result<AgentMetadataToml, StoreError> {
        let content = fs::read_to_string(self.agent_toml_path())?;
        Ok(toml::from_str::<AgentMetadataToml>(&content)?)
    }

    /// Loads typed `POLICY.toml` metadata.
    pub fn load_policy(&self) -> Result<AgentPolicyToml, StoreError> {
        let content = fs::read_to_string(self.policy_toml_path())?;
        Ok(toml::from_str::<AgentPolicyToml>(&content)?)
    }

    /// Runs missing/corrupt/oversized checks for this agent home.
    pub fn doctor(&self, options: &AgentHomeDoctorOptions) -> Vec<AgentHomeDiagnostic> {
        let mut diagnostics = Vec::new();
        diagnostics.extend(self.toml_check(
            "agent.AGENT.toml",
            &self.agent_toml_path(),
            options.max_agent_toml_bytes,
            |content| toml::from_str::<AgentMetadataToml>(content).map(|_| ()),
        ));
        diagnostics.extend(self.toml_check(
            "agent.POLICY.toml",
            &self.policy_toml_path(),
            options.max_policy_toml_bytes,
            |content| toml::from_str::<AgentPolicyToml>(content).map(|_| ()),
        ));
        diagnostics.extend(self.toml_check(
            "agent.SKILL_POLICY.toml",
            &self.root.join("SKILL_POLICY.toml"),
            options.max_policy_toml_bytes,
            |content| toml::from_str::<SkillPolicyToml>(content).map(|_| ()),
        ));

        for file_name in ["PERSONA.md", "INSTRUCTIONS.md", "USER.md", "TOOLS.md"] {
            diagnostics.push(self.text_file_check(
                &format!("agent.{file_name}"),
                &self.root.join(file_name),
                options.max_text_file_bytes,
            ));
        }
        diagnostics.push(self.text_file_check(
            "agent.MEMORY.md",
            &self.root.join("MEMORY.md"),
            options.max_memory_file_bytes,
        ));
        diagnostics.push(self.text_file_check(
            "agent.memory.decisions",
            &self.root.join("memory/decisions.md"),
            options.max_memory_file_bytes,
        ));
        diagnostics.push(self.text_file_check(
            "agent.memory.delegation",
            &self.root.join("memory/delegation.md"),
            options.max_memory_file_bytes,
        ));

        diagnostics
    }

    fn toml_check(
        &self,
        name: &str,
        path: &Path,
        max_bytes: u64,
        parse: impl FnOnce(&str) -> Result<(), toml::de::Error>,
    ) -> Vec<AgentHomeDiagnostic> {
        let mut diagnostics = Vec::new();
        match fs::metadata(path) {
            Ok(metadata) if !metadata.is_file() => {
                diagnostics.push(self.diagnostic(
                    name,
                    "error",
                    format!("{} is not a file", path.display()),
                ));
                return diagnostics;
            }
            Ok(metadata) => {
                if metadata.len() > max_bytes {
                    diagnostics.push(self.diagnostic(
                        name,
                        "warn",
                        format!(
                            "{} is {} bytes; limit is {max_bytes}",
                            path.display(),
                            metadata.len()
                        ),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                diagnostics.push(self.diagnostic(
                    name,
                    "error",
                    format!("{} is missing", path.display()),
                ));
                return diagnostics;
            }
            Err(error) => {
                diagnostics.push(self.diagnostic(
                    name,
                    "error",
                    format!("could not stat {}: {error}", path.display()),
                ));
                return diagnostics;
            }
        }

        match fs::read_to_string(path) {
            Ok(content) => match parse(&content) {
                Ok(()) => diagnostics.push(self.diagnostic(
                    name,
                    "ok",
                    format!("{} is valid", path.display()),
                )),
                Err(error) => diagnostics.push(self.diagnostic(
                    name,
                    "error",
                    format!("{} is invalid TOML: {error}", path.display()),
                )),
            },
            Err(error) => diagnostics.push(self.diagnostic(
                name,
                "error",
                format!("could not read {}: {error}", path.display()),
            )),
        }
        diagnostics
    }

    fn text_file_check(&self, name: &str, path: &Path, max_bytes: u64) -> AgentHomeDiagnostic {
        match fs::metadata(path) {
            Ok(metadata) if !metadata.is_file() => {
                self.diagnostic(name, "error", format!("{} is not a file", path.display()))
            }
            Ok(metadata) if metadata.len() > max_bytes => self.diagnostic(
                name,
                "warn",
                format!(
                    "{} is {} bytes; limit is {max_bytes}",
                    path.display(),
                    metadata.len()
                ),
            ),
            Ok(_) => self.diagnostic(name, "ok", format!("{} exists", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.diagnostic(name, "warn", format!("{} is missing", path.display()))
            }
            Err(error) => self.diagnostic(
                name,
                "error",
                format!("could not stat {}: {error}", path.display()),
            ),
        }
    }

    fn diagnostic(
        &self,
        name: &str,
        status: impl Into<String>,
        message: impl Into<String>,
    ) -> AgentHomeDiagnostic {
        AgentHomeDiagnostic {
            name: format!("{}/{}", self.agent_id, name),
            status: status.into(),
            message: message.into(),
        }
    }
}

/// Template metadata used to initialize an agent home.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentHomeTemplate {
    /// Stable agent ID.
    pub agent_id: AgentId,
    /// User-visible agent name.
    pub display_name: String,
    /// Agent role.
    pub role: String,
    /// Parent agent ID when this is a child/subagent.
    pub parent_agent_id: Option<AgentId>,
    /// Default model selection.
    pub model: String,
}

impl AgentHomeTemplate {
    /// Creates a template descriptor.
    pub fn new(
        agent_id: AgentId,
        display_name: impl Into<String>,
        role: impl Into<String>,
        parent_agent_id: Option<AgentId>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            agent_id,
            display_name: display_name.into(),
            role: role.into(),
            parent_agent_id,
            model: model.into(),
        }
    }
}

/// Typed `AGENT.toml` metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentMetadataToml {
    /// Agent identity and display metadata.
    pub agent: AgentIdentityToml,
    /// Conventional files inside the agent home.
    pub files: AgentFilesToml,
}

/// Identity section for `AGENT.toml`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentIdentityToml {
    /// Stable agent ID.
    pub id: String,
    /// User-visible agent name.
    pub display_name: String,
    /// Runtime role.
    pub role: String,
    /// Optional parent agent ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    /// Default model selection.
    pub model: String,
}

impl Default for AgentIdentityToml {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            role: String::new(),
            parent_agent_id: None,
            model: "auto".to_owned(),
        }
    }
}

/// Conventional file paths section for `AGENT.toml`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentFilesToml {
    /// Persona markdown path.
    pub persona: PathBuf,
    /// Runtime instructions markdown path.
    pub instructions: PathBuf,
    /// User preferences markdown path.
    pub user: PathBuf,
    /// Agent memory markdown path.
    pub memory: PathBuf,
    /// Tool guidance markdown path.
    pub tools: PathBuf,
    /// Machine-readable policy TOML path.
    pub policy: PathBuf,
    /// Machine-readable skill policy TOML path.
    pub skill_policy: PathBuf,
}

impl Default for AgentFilesToml {
    fn default() -> Self {
        Self {
            persona: PathBuf::from("PERSONA.md"),
            instructions: PathBuf::from("INSTRUCTIONS.md"),
            user: PathBuf::from("USER.md"),
            memory: PathBuf::from("MEMORY.md"),
            tools: PathBuf::from("TOOLS.md"),
            policy: PathBuf::from("POLICY.toml"),
            skill_policy: PathBuf::from("SKILL_POLICY.toml"),
        }
    }
}

/// Typed `POLICY.toml` metadata. Runtime enforcement remains daemon/policy-owned.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentPolicyToml {
    /// Policy capability defaults.
    pub policy: AgentPolicyDefaultsToml,
    /// Sandbox path defaults.
    pub sandbox: AgentSandboxToml,
    /// File-size guardrails for profile/agent doctor checks.
    pub limits: AgentPolicyLimitsToml,
}

/// Policy defaults section for `POLICY.toml`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentPolicyDefaultsToml {
    /// Schema version for future migrations.
    pub version: u32,
    /// Default workspace access metadata.
    pub default_workspace_access: Vec<WorkspaceAccess>,
    /// Whether network-capable tools are allowed without additional policy.
    pub allow_network: bool,
    /// Whether secret access is allowed without additional policy.
    pub allow_secret_access: bool,
    /// Whether system-changing tools are allowed without additional policy.
    pub allow_system_change: bool,
    /// Whether non-safe actions require approval.
    pub approval_required: bool,
}

impl Default for AgentPolicyDefaultsToml {
    fn default() -> Self {
        Self {
            version: 1,
            default_workspace_access: vec![WorkspaceAccess::Read],
            allow_network: false,
            allow_secret_access: false,
            allow_system_change: false,
            approval_required: true,
        }
    }
}

/// Sandbox defaults section for `POLICY.toml`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentSandboxToml {
    /// Default sandbox mode label.
    pub default: String,
    /// Metadata denied paths. Track D/tool runtime owns enforcement.
    pub denied_paths: Vec<PathBuf>,
}

impl Default for AgentSandboxToml {
    fn default() -> Self {
        Self {
            default: "workspace".to_owned(),
            denied_paths: default_agent_denied_paths(),
        }
    }
}

/// Doctor guardrail limits section for `POLICY.toml`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentPolicyLimitsToml {
    /// Maximum `AGENT.toml` size.
    pub max_agent_toml_bytes: u64,
    /// Maximum `POLICY.toml` size.
    pub max_policy_toml_bytes: u64,
    /// Maximum persona/instruction/user/tool guidance file size.
    pub max_text_file_bytes: u64,
    /// Maximum memory markdown file size.
    pub max_memory_file_bytes: u64,
}

impl Default for AgentPolicyLimitsToml {
    fn default() -> Self {
        Self {
            max_agent_toml_bytes: 64 * 1024,
            max_policy_toml_bytes: 64 * 1024,
            max_text_file_bytes: 128 * 1024,
            max_memory_file_bytes: 1024 * 1024,
        }
    }
}

/// Typed `SKILL_POLICY.toml` metadata.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SkillPolicyToml {
    /// Whether agent-local skills are active.
    pub enabled: bool,
    /// Skill precedence labels for this agent home.
    pub precedence: Vec<String>,
    /// Whether generated skill candidates require review.
    pub require_review: bool,
}

impl Default for SkillPolicyToml {
    fn default() -> Self {
        Self {
            enabled: true,
            precedence: vec![
                "agent".to_owned(),
                "workspace".to_owned(),
                "profile".to_owned(),
            ],
            require_review: true,
        }
    }
}

/// Agent-home doctor options.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentHomeDoctorOptions {
    /// Maximum `AGENT.toml` size.
    pub max_agent_toml_bytes: u64,
    /// Maximum `POLICY.toml` size.
    pub max_policy_toml_bytes: u64,
    /// Maximum persona/instruction/user/tool guidance file size.
    pub max_text_file_bytes: u64,
    /// Maximum memory markdown file size.
    pub max_memory_file_bytes: u64,
}

impl Default for AgentHomeDoctorOptions {
    fn default() -> Self {
        let limits = AgentPolicyLimitsToml::default();
        Self {
            max_agent_toml_bytes: limits.max_agent_toml_bytes,
            max_policy_toml_bytes: limits.max_policy_toml_bytes,
            max_text_file_bytes: limits.max_text_file_bytes,
            max_memory_file_bytes: limits.max_memory_file_bytes,
        }
    }
}

/// One profile/agent-home doctor diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentHomeDiagnostic {
    /// Check name.
    pub name: String,
    /// `ok`, `warn`, or `error`.
    pub status: String,
    /// Human-readable diagnostic.
    pub message: String,
}

/// Conventional worker artifact paths under a profile home.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerArtifactPathSet {
    /// Worker artifact root.
    pub root: PathBuf,
    /// Patch/diff artifact path.
    pub patch: PathBuf,
    /// Test report artifact path.
    pub test_report: PathBuf,
    /// Worker summary artifact path.
    pub summary: PathBuf,
    /// Changed-files manifest path.
    pub changed_files: PathBuf,
    /// Memory candidate JSONL path.
    pub memory_candidates: PathBuf,
}

impl WorkerArtifactPathSet {
    /// Creates conventional artifact paths below `artifacts/workers/<worker-id>`.
    pub fn new(root: impl AsRef<Path>, worker_id: impl AsRef<str>) -> Self {
        let root = root.as_ref().join(safe_file_stem(worker_id.as_ref()));
        Self {
            patch: root.join("patch.diff"),
            test_report: root.join("test-report.json"),
            summary: root.join("summary.md"),
            changed_files: root.join("changed-files.json"),
            memory_candidates: root.join("memory-candidates.jsonl"),
            root,
        }
    }
}

/// Profile-local workspace registry stored as TOML.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceRegistry {
    /// Registered project/document/sandbox workspaces.
    #[serde(default)]
    pub workspace: Vec<WorkspaceMetadata>,
}

/// Metadata for one registered execution workspace.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkspaceMetadata {
    /// Stable workspace ID.
    pub id: String,
    /// Workspace category.
    pub kind: WorkspaceKind,
    /// Workspace root path. `~` is expanded by helper APIs after loading.
    pub root: PathBuf,
    /// Version-control backend.
    pub vcs: WorkspaceVcs,
    /// Human or profile owner label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Whether CADIS can treat this root as a trusted project root.
    pub trusted: bool,
    /// Relative directory for worker worktrees inside a project root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<PathBuf>,
    /// Relative directory for workspace-local artifacts inside a project root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_root: Option<PathBuf>,
    /// Checkpoint behavior for this workspace.
    pub checkpoint_policy: CheckpointPolicy,
    /// Inline aliases for detection and routing.
    #[serde(default, rename = "alias", skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<WorkspaceAlias>,
}

impl Default for WorkspaceMetadata {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: WorkspaceKind::Project,
            root: PathBuf::new(),
            vcs: WorkspaceVcs::None,
            owner: None,
            trusted: false,
            worktree_root: Some(PathBuf::from(".cadis/worktrees")),
            artifact_root: Some(PathBuf::from(".cadis/artifacts")),
            checkpoint_policy: CheckpointPolicy::Disabled,
            aliases: Vec::new(),
        }
    }
}

impl WorkspaceMetadata {
    /// Returns this workspace with `~` expanded in path fields.
    pub fn expanded_paths(mut self) -> Self {
        self.root = expand_home(&self.root);
        self
    }
}

/// Workspace category persisted in the profile registry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// Source project workspace.
    #[default]
    Project,
    /// User document collection.
    Documents,
    /// Sandbox or temporary workspace root.
    Sandbox,
    /// Git worktree workspace.
    Worktree,
}

/// Workspace VCS backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceVcs {
    /// No VCS backend.
    #[default]
    None,
    /// Git repository or worktree.
    Git,
}

/// Workspace checkpoint policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointPolicy {
    /// Checkpoints enabled before mutating operations.
    Enabled,
    /// Checkpoints disabled for this root.
    #[default]
    Disabled,
}

/// Alias metadata for one registered workspace.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkspaceAlias {
    /// Workspace ID this alias group targets.
    pub workspace_id: String,
    /// Alias strings.
    pub aliases: Vec<String>,
}

/// Profile-local workspace registry TOML helper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRegistryStore {
    profile_home: ProfileHome,
}

impl WorkspaceRegistryStore {
    /// Creates a registry helper for a profile home.
    pub fn new(profile_home: ProfileHome) -> Self {
        Self { profile_home }
    }

    /// Returns the registry file path.
    pub fn path(&self) -> PathBuf {
        self.profile_home.workspace_registry_path()
    }

    /// Loads the workspace registry, returning an empty registry when missing.
    pub fn load(&self) -> Result<WorkspaceRegistry, StoreError> {
        self.profile_home.ensure_layout()?;
        let path = self.path();
        if !path.exists() {
            return Ok(WorkspaceRegistry::default());
        }

        let content = fs::read_to_string(path)?;
        let mut registry = toml::from_str::<WorkspaceRegistry>(&content)?;
        for workspace in &mut registry.workspace {
            workspace.root = expand_home(&workspace.root);
        }
        Ok(registry)
    }

    /// Atomically writes the registry as redacted TOML.
    pub fn save(&self, registry: &WorkspaceRegistry) -> Result<(), StoreError> {
        self.profile_home.ensure_layout()?;
        let mut toml = redact(&toml::to_string_pretty(registry)?);
        if !toml.ends_with('\n') {
            toml.push('\n');
        }
        atomic_write_private_file(&self.path(), toml.as_bytes())
    }
}

/// Project-local `.cadis/workspace.toml` metadata.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ProjectWorkspaceMetadata {
    /// Workspace ID expected to match the profile registry entry.
    pub workspace_id: String,
    /// Workspace category.
    pub kind: WorkspaceKind,
    /// Version-control backend.
    pub vcs: WorkspaceVcs,
    /// Relative directory for CADIS worker worktrees.
    pub worktree_root: PathBuf,
    /// Relative directory for workspace-local artifacts.
    pub artifact_root: PathBuf,
    /// Relative directory for project-scoped media assets.
    pub media_root: PathBuf,
}

impl Default for ProjectWorkspaceMetadata {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            kind: WorkspaceKind::Project,
            vcs: WorkspaceVcs::None,
            worktree_root: PathBuf::from(".cadis/worktrees"),
            artifact_root: PathBuf::from(".cadis/artifacts"),
            media_root: PathBuf::from(".cadis/media"),
        }
    }
}

/// Project-local `.cadis/` metadata helper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectWorkspaceStore {
    root: PathBuf,
}

impl ProjectWorkspaceStore {
    /// Creates a project metadata helper rooted at a project workspace.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Returns the project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the project `.cadis` directory path.
    pub fn cadis_dir(&self) -> PathBuf {
        self.root.join(".cadis")
    }

    /// Returns the project `workspace.toml` path.
    pub fn workspace_toml_path(&self) -> PathBuf {
        self.cadis_dir().join("workspace.toml")
    }

    /// Returns the project worktree root from metadata or the default convention.
    pub fn worktree_root(&self, metadata: Option<&ProjectWorkspaceMetadata>) -> PathBuf {
        let root = metadata
            .map(|metadata| metadata.worktree_root.as_path())
            .unwrap_or_else(|| Path::new(".cadis/worktrees"));
        self.project_relative_path(root)
    }

    /// Returns the project-local worktree metadata directory.
    pub fn worktree_metadata_dir(&self, metadata: Option<&ProjectWorkspaceMetadata>) -> PathBuf {
        self.worktree_root(metadata).join(".metadata")
    }

    /// Returns worker-specific worktree and metadata paths.
    pub fn worker_worktree_paths(
        &self,
        worker_id: impl AsRef<str>,
        metadata: Option<&ProjectWorkspaceMetadata>,
    ) -> ProjectWorkerWorktreePaths {
        let worker_id = safe_file_stem(worker_id.as_ref());
        let worktree_root = self.worktree_root(metadata);
        let metadata_dir = self.worktree_metadata_dir(metadata);
        ProjectWorkerWorktreePaths {
            worker_id: worker_id.clone(),
            worktree_root: worktree_root.clone(),
            worktree_path: worktree_root.join(&worker_id),
            metadata_path: metadata_dir.join(format!("{worker_id}.toml")),
        }
    }

    /// Ensures the non-secret project `.cadis` metadata skeleton exists.
    pub fn ensure_layout(&self) -> Result<(), StoreError> {
        fs::create_dir_all(self.cadis_dir())?;
        for path in ["worktrees", "worktrees/.metadata", "artifacts", "media"] {
            fs::create_dir_all(self.cadis_dir().join(path))?;
        }
        write_template_file_if_missing(
            &self.cadis_dir().join(".gitignore"),
            project_gitignore_template(),
        )?;
        Ok(())
    }

    /// Loads project-local workspace metadata. Missing metadata returns `Ok(None)`.
    pub fn load(&self) -> Result<Option<ProjectWorkspaceMetadata>, StoreError> {
        let path = self.workspace_toml_path();
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        Ok(Some(toml::from_str::<ProjectWorkspaceMetadata>(&content)?))
    }

    /// Writes project-local workspace metadata with redaction and atomic replace.
    pub fn save(&self, metadata: &ProjectWorkspaceMetadata) -> Result<(), StoreError> {
        self.ensure_layout()?;
        let mut toml = redact(&toml::to_string_pretty(metadata)?);
        if !toml.ends_with('\n') {
            toml.push('\n');
        }
        atomic_write_private_file(&self.workspace_toml_path(), toml.as_bytes())
    }

    /// Writes one project-local worker worktree metadata file.
    pub fn save_worker_worktree_metadata(
        &self,
        metadata: &ProjectWorkerWorktreeMetadata,
    ) -> Result<(), StoreError> {
        let workspace_metadata = self.load()?;
        let paths = self.worker_worktree_paths(&metadata.worker_id, workspace_metadata.as_ref());
        let mut toml = redact(&toml::to_string_pretty(metadata)?);
        if !toml.ends_with('\n') {
            toml.push('\n');
        }
        atomic_write_private_file(&paths.metadata_path, toml.as_bytes())
    }

    /// Loads one project-local worker worktree metadata file.
    pub fn load_worker_worktree_metadata(
        &self,
        worker_id: impl AsRef<str>,
    ) -> Result<Option<ProjectWorkerWorktreeMetadata>, StoreError> {
        let workspace_metadata = self.load()?;
        let paths = self.worker_worktree_paths(worker_id, workspace_metadata.as_ref());
        if !paths.metadata_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(paths.metadata_path)?;
        Ok(Some(toml::from_str::<ProjectWorkerWorktreeMetadata>(
            &content,
        )?))
    }

    /// Recovers project-local worker worktree metadata and reports invalid TOML.
    pub fn recover_worker_worktree_metadata(
        &self,
    ) -> Result<ProjectWorkerWorktreeRecovery, StoreError> {
        let workspace_metadata = self.load()?;
        let metadata_dir = self.worktree_metadata_dir(workspace_metadata.as_ref());
        if !metadata_dir.exists() {
            return Ok(ProjectWorkerWorktreeRecovery::default());
        }

        let mut records = Vec::new();
        let mut diagnostics = Vec::new();
        for entry in fs::read_dir(metadata_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("toml")
            {
                continue;
            }

            match fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<ProjectWorkerWorktreeMetadata>(&content) {
                    Ok(metadata) => records.push(ProjectWorkerWorktreeRecord { path, metadata }),
                    Err(error) => diagnostics.push(ProjectWorktreeDiagnostic {
                        name: "workspace.worktrees.metadata".to_owned(),
                        status: "error".to_owned(),
                        message: format!("{} is invalid TOML: {error}", path.display()),
                    }),
                },
                Err(error) => diagnostics.push(ProjectWorktreeDiagnostic {
                    name: "workspace.worktrees.metadata".to_owned(),
                    status: "error".to_owned(),
                    message: format!("could not read {}: {error}", path.display()),
                }),
            }
        }

        Ok(ProjectWorkerWorktreeRecovery {
            records,
            diagnostics,
        })
    }

    /// Runs stale project worktree metadata and artifact-root diagnostics.
    pub fn worker_worktree_diagnostics(
        &self,
    ) -> Result<Vec<ProjectWorktreeDiagnostic>, StoreError> {
        let mut recovery = self.recover_worker_worktree_metadata()?;
        let mut diagnostics = Vec::new();
        diagnostics.append(&mut recovery.diagnostics);

        diagnostics.push(ProjectWorktreeDiagnostic {
            name: "workspace.worktrees.metadata".to_owned(),
            status: "ok".to_owned(),
            message: format!(
                "{} worker worktree metadata record(s)",
                recovery.records.len()
            ),
        });

        for record in recovery.records {
            let worker_id = safe_file_stem(&record.metadata.worker_id);
            let worktree_path = self.project_relative_path(&record.metadata.worktree_path);
            diagnostics.push(path_diagnostic(
                &format!("workspace.worktrees.{worker_id}.path"),
                &worktree_path,
                format!("worker worktree path from {}", record.path.display()),
            ));

            let artifact_root = self.project_relative_path(&record.metadata.artifact_root);
            diagnostics.push(path_diagnostic(
                &format!("workspace.worktrees.{worker_id}.artifacts"),
                &artifact_root,
                "worker artifact root".to_owned(),
            ));
        }

        Ok(diagnostics)
    }

    fn project_relative_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

/// Worker-specific project worktree path set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectWorkerWorktreePaths {
    /// Redaction-safe worker ID path component.
    pub worker_id: String,
    /// Root directory where CADIS project worktrees live.
    pub worktree_root: PathBuf,
    /// Worker-specific worktree directory.
    pub worktree_path: PathBuf,
    /// Project-local metadata TOML path for this worker.
    pub metadata_path: PathBuf,
}

/// Project-local worker worktree metadata stored below `.cadis/worktrees/.metadata/`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectWorkerWorktreeMetadata {
    /// Worker ID.
    pub worker_id: String,
    /// Workspace registry ID.
    pub workspace_id: String,
    /// Planned or actual worktree path. Relative paths resolve against the project root.
    pub worktree_path: PathBuf,
    /// Intended or actual branch name.
    pub branch_name: String,
    /// Base ref for branch creation, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Current worktree metadata lifecycle state.
    pub state: ProjectWorkerWorktreeState,
    /// Profile-scoped worker artifact root. Relative paths resolve against the project root.
    pub artifact_root: PathBuf,
}

/// Project-local worker worktree metadata lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectWorkerWorktreeState {
    /// Worktree has been planned but not created.
    Planned,
    /// Worktree exists and is assigned to the worker.
    Ready,
    /// Worktree is retained for user review or patch application.
    ReviewPending,
    /// Worktree cleanup has been requested but files have not been removed.
    CleanupPending,
    /// Worktree has been removed while metadata remains for diagnostics/audit.
    Removed,
}

/// Recovered project worker worktree metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectWorkerWorktreeRecovery {
    /// Valid metadata records.
    pub records: Vec<ProjectWorkerWorktreeRecord>,
    /// Invalid TOML diagnostics.
    pub diagnostics: Vec<ProjectWorktreeDiagnostic>,
}

/// One recovered project worker worktree metadata record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectWorkerWorktreeRecord {
    /// Metadata TOML path.
    pub path: PathBuf,
    /// Parsed metadata.
    pub metadata: ProjectWorkerWorktreeMetadata,
}

/// Project worktree doctor diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectWorktreeDiagnostic {
    /// Check name.
    pub name: String,
    /// `ok`, `warn`, or `error`.
    pub status: String,
    /// Human-readable diagnostic.
    pub message: String,
}

/// Workspace access level granted to an agent or worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccess {
    /// Read-only access.
    Read,
    /// File mutation access.
    Write,
    /// Shell/process execution access.
    Exec,
    /// Administrative workspace operations.
    Admin,
}

/// Source that created a workspace grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    /// Channel or routing rule.
    Route,
    /// Explicit user approval.
    User,
    /// Policy engine decision.
    Policy,
    /// Worker spawn flow.
    WorkerSpawn,
}

/// Append-only workspace grant record persisted under profile `workspaces/grants.jsonl`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceGrantRecord {
    /// Stable grant ID.
    pub grant_id: String,
    /// Profile ID.
    pub profile_id: String,
    /// Agent receiving the grant. Missing means the default local runtime context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Workspace ID from the profile registry.
    pub workspace_id: String,
    /// Granted root path.
    pub root: PathBuf,
    /// Granted access levels.
    pub access: Vec<WorkspaceAccess>,
    /// Grant creation time.
    pub created_at: Timestamp,
    /// Optional expiration time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    /// Grant source.
    pub source: GrantSource,
    /// Redacted human-readable reason or route note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Profile-local workspace grant JSONL helper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceGrantStore {
    profile_home: ProfileHome,
}

impl WorkspaceGrantStore {
    /// Creates a grant helper for a profile home.
    pub fn new(profile_home: ProfileHome) -> Self {
        Self { profile_home }
    }

    /// Returns the JSONL grant path.
    pub fn path(&self) -> PathBuf {
        self.profile_home.workspace_grants_path()
    }

    /// Appends one redacted grant record.
    pub fn append(&self, record: &WorkspaceGrantRecord) -> Result<(), StoreError> {
        self.profile_home.ensure_layout()?;
        let mut line = redact(&serde_json::to_string(record)?);
        line.push('\n');
        let path = self.path();
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        set_private_file_permissions(&path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    /// Rewrites the active grant set as redacted JSONL.
    pub fn replace_all(&self, records: &[WorkspaceGrantRecord]) -> Result<(), StoreError> {
        self.profile_home.ensure_layout()?;
        let mut content = String::new();
        for record in records {
            content.push_str(&redact(&serde_json::to_string(record)?));
            content.push('\n');
        }
        atomic_write_private_file(&self.path(), content.as_bytes())
    }

    /// Loads valid grant records and reports invalid JSONL lines as diagnostics.
    pub fn load(&self) -> Result<WorkspaceGrantRecovery, StoreError> {
        self.profile_home.ensure_layout()?;
        let path = self.path();
        if !path.exists() {
            return Ok(WorkspaceGrantRecovery::default());
        }

        let content = fs::read_to_string(&path)?;
        let mut records = Vec::new();
        let mut diagnostics = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<WorkspaceGrantRecord>(line) {
                Ok(record) => records.push(record),
                Err(error) => diagnostics.push(WorkspaceGrantDiagnostic {
                    line: index + 1,
                    reason: format!("invalid workspace grant JSON: {error}"),
                }),
            }
        }

        Ok(WorkspaceGrantRecovery {
            records,
            diagnostics,
        })
    }
}

/// Recovered workspace grant records and load diagnostics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceGrantRecovery {
    /// Valid grant records.
    pub records: Vec<WorkspaceGrantRecord>,
    /// Invalid JSONL lines skipped during recovery.
    pub diagnostics: Vec<WorkspaceGrantDiagnostic>,
}

/// Diagnostic for one invalid grant JSONL line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceGrantDiagnostic {
    /// One-based line number.
    pub line: usize,
    /// Parse failure reason.
    pub reason: String,
}

/// Persisted approval lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// Approval is waiting for a response.
    Pending,
    /// Approval has been resolved by a client response.
    Resolved,
    /// Approval expired before it could be approved.
    Expired,
}

/// Persisted approval request and resolution record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ApprovalRecord {
    /// Approval ID.
    pub approval_id: ApprovalId,
    /// Session ID.
    pub session_id: SessionId,
    /// Tool call ID guarded by this approval.
    pub tool_call_id: ToolCallId,
    /// Tool name guarded by this approval.
    pub tool_name: String,
    /// Risk class assigned by policy.
    pub risk_class: RiskClass,
    /// UI title.
    pub title: String,
    /// Redacted summary.
    pub summary: String,
    /// Optional redacted command or operation details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Optional workspace or cwd.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Request timestamp.
    pub requested_at: Timestamp,
    /// Expiration timestamp.
    pub expires_at: Timestamp,
    /// Current state.
    pub state: ApprovalState,
    /// Final decision, when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<ApprovalDecision>,
    /// Redacted resolver reason, when supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Resolution timestamp, when resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<Timestamp>,
}

/// Durable approval record store rooted under `~/.cadis/state/approvals`.
#[derive(Clone, Debug)]
pub struct ApprovalStore {
    approvals_dir: PathBuf,
}

impl ApprovalStore {
    /// Creates an approval store rooted under CADIS home.
    pub fn new(cadis_home: impl AsRef<Path>) -> Self {
        Self {
            approvals_dir: cadis_home.as_ref().join("state").join("approvals"),
        }
    }

    /// Saves one redacted approval record.
    pub fn save(&self, record: &ApprovalRecord) -> Result<(), StoreError> {
        create_private_dir(&self.approvals_dir)?;
        let mut json = redact(&serde_json::to_string_pretty(record)?);
        json.push('\n');
        let path = self.approval_path(&record.approval_id);
        let tmp_path = self.temporary_path(&record.approval_id);

        let write_result = (|| -> Result<(), StoreError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            set_private_file_permissions(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp_path, &path)?;
            sync_parent_dir(&self.approvals_dir)?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_result
    }

    /// Loads one approval record by ID.
    pub fn load(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        let path = self.approval_path(approval_id);
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&content)?))
    }

    fn approval_path(&self, approval_id: &ApprovalId) -> PathBuf {
        self.approvals_dir
            .join(format!("{}.json", safe_file_stem(approval_id.as_str())))
    }

    fn temporary_path(&self, approval_id: &ApprovalId) -> PathBuf {
        let process = std::process::id();
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        self.approvals_dir.join(format!(
            ".{}.json.tmp.{process}.{counter}.{nanos}",
            safe_file_stem(approval_id.as_str())
        ))
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
        "state",
        "state/sessions",
        "state/agents",
        "state/agent-sessions",
        "state/workers",
        "state/approvals",
    ] {
        create_private_dir(&config.cadis_home.join(path))?;
    }

    CadisHome::new(&config.cadis_home).init_profile(&config.profile.default_profile)?;

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
///
/// On Windows this returns `None` because the daemon uses TCP instead of Unix sockets.
/// On Linux/macOS: `$XDG_RUNTIME_DIR/cadis/cadisd.sock` or `<cadis_home>/run/cadisd.sock`.
pub fn default_socket_path(cadis_home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let _ = cadis_home;
        None
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
            if !runtime_dir.trim().is_empty() {
                return Some(PathBuf::from(runtime_dir).join("cadis").join("cadisd.sock"));
            }
        }

        Some(cadis_home.join("run").join("cadisd.sock"))
    }
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

/// Durable JSON state helper rooted under `~/.cadis/state`.
#[derive(Clone, Debug)]
pub struct StateStore {
    state_dir: PathBuf,
}

impl StateStore {
    /// Creates a durable state helper rooted under the configured CADIS home.
    pub fn new(config: &CadisConfig) -> Self {
        Self {
            state_dir: config.cadis_home.join("state"),
        }
    }

    /// Returns the root durable state directory.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Ensures all durable state directories exist with private permissions.
    pub fn ensure_layout(&self) -> Result<(), StoreError> {
        for kind in StateKind::all() {
            create_private_dir(&self.kind_dir(kind))?;
        }
        Ok(())
    }

    /// Returns the session metadata file path for a session ID.
    pub fn session_path(&self, session_id: &SessionId) -> PathBuf {
        self.metadata_path(StateKind::Session, session_id.as_str())
    }

    /// Returns the agent metadata file path for an agent ID.
    pub fn agent_path(&self, agent_id: &AgentId) -> PathBuf {
        self.metadata_path(StateKind::Agent, agent_id.as_str())
    }

    /// Returns the AgentSession metadata file path for an AgentSession ID.
    pub fn agent_session_path(&self, agent_session_id: &AgentSessionId) -> PathBuf {
        self.metadata_path(StateKind::AgentSession, agent_session_id.as_str())
    }

    /// Returns the worker metadata file path for a worker ID.
    pub fn worker_path(&self, worker_id: &str) -> PathBuf {
        self.metadata_path(StateKind::Worker, worker_id)
    }

    /// Returns the approval metadata file path for an approval ID.
    pub fn approval_path(&self, approval_id: &ApprovalId) -> PathBuf {
        self.metadata_path(StateKind::Approval, approval_id.as_str())
    }

    /// Atomically writes one redacted session metadata JSON file.
    pub fn write_session_metadata<T: Serialize>(
        &self,
        session_id: &SessionId,
        metadata: &T,
    ) -> Result<(), StoreError> {
        self.write_metadata(StateKind::Session, session_id.as_str(), metadata)
    }

    /// Atomically writes one redacted agent metadata JSON file.
    pub fn write_agent_metadata<T: Serialize>(
        &self,
        agent_id: &AgentId,
        metadata: &T,
    ) -> Result<(), StoreError> {
        self.write_metadata(StateKind::Agent, agent_id.as_str(), metadata)
    }

    /// Atomically writes one redacted AgentSession metadata JSON file.
    pub fn write_agent_session_metadata<T: Serialize>(
        &self,
        agent_session_id: &AgentSessionId,
        metadata: &T,
    ) -> Result<(), StoreError> {
        self.write_metadata(StateKind::AgentSession, agent_session_id.as_str(), metadata)
    }

    /// Atomically writes one redacted worker metadata JSON file.
    pub fn write_worker_metadata<T: Serialize>(
        &self,
        worker_id: &str,
        metadata: &T,
    ) -> Result<(), StoreError> {
        self.write_metadata(StateKind::Worker, worker_id, metadata)
    }

    /// Atomically writes one redacted approval metadata JSON file.
    pub fn write_approval_metadata<T: Serialize>(
        &self,
        approval_id: &ApprovalId,
        metadata: &T,
    ) -> Result<(), StoreError> {
        self.write_metadata(StateKind::Approval, approval_id.as_str(), metadata)
    }

    /// Removes one persisted session metadata JSON file when present.
    pub fn remove_session_metadata(&self, session_id: &SessionId) -> Result<(), StoreError> {
        self.remove_metadata(StateKind::Session, session_id.as_str())
    }

    /// Removes one persisted agent metadata JSON file when present.
    pub fn remove_agent_metadata(&self, agent_id: &AgentId) -> Result<(), StoreError> {
        self.remove_metadata(StateKind::Agent, agent_id.as_str())
    }

    /// Removes one persisted AgentSession metadata JSON file when present.
    pub fn remove_agent_session_metadata(
        &self,
        agent_session_id: &AgentSessionId,
    ) -> Result<(), StoreError> {
        self.remove_metadata(StateKind::AgentSession, agent_session_id.as_str())
    }

    /// Removes one persisted worker metadata JSON file when present.
    pub fn remove_worker_metadata(&self, worker_id: &str) -> Result<(), StoreError> {
        self.remove_metadata(StateKind::Worker, worker_id)
    }

    /// Removes one persisted approval metadata JSON file when present.
    pub fn remove_approval_metadata(&self, approval_id: &ApprovalId) -> Result<(), StoreError> {
        self.remove_metadata(StateKind::Approval, approval_id.as_str())
    }

    /// Recovers valid session metadata files and reports invalid files as diagnostics.
    pub fn recover_session_metadata<T: DeserializeOwned>(
        &self,
    ) -> Result<StateRecovery<T>, StoreError> {
        self.recover_metadata(StateKind::Session)
    }

    /// Recovers valid agent metadata files and reports invalid files as diagnostics.
    pub fn recover_agent_metadata<T: DeserializeOwned>(
        &self,
    ) -> Result<StateRecovery<T>, StoreError> {
        self.recover_metadata(StateKind::Agent)
    }

    /// Recovers valid AgentSession metadata files and reports invalid files as diagnostics.
    pub fn recover_agent_session_metadata<T: DeserializeOwned>(
        &self,
    ) -> Result<StateRecovery<T>, StoreError> {
        self.recover_metadata(StateKind::AgentSession)
    }

    /// Recovers valid worker metadata files and reports invalid files as diagnostics.
    pub fn recover_worker_metadata<T: DeserializeOwned>(
        &self,
    ) -> Result<StateRecovery<T>, StoreError> {
        self.recover_metadata(StateKind::Worker)
    }

    /// Recovers valid approval metadata files and reports invalid files as diagnostics.
    pub fn recover_approval_metadata<T: DeserializeOwned>(
        &self,
    ) -> Result<StateRecovery<T>, StoreError> {
        self.recover_metadata(StateKind::Approval)
    }

    fn write_metadata<T: Serialize>(
        &self,
        kind: StateKind,
        id: &str,
        metadata: &T,
    ) -> Result<(), StoreError> {
        let dir = self.kind_dir(kind);
        create_private_dir(&dir)?;

        let path = self.metadata_path(kind, id);
        let tmp_path = self.temporary_path(kind, id);
        let mut json = redact(&serde_json::to_string_pretty(metadata)?);
        json.push('\n');

        let write_result = (|| -> Result<(), StoreError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            set_private_file_permissions(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp_path, &path)?;
            sync_parent_dir(&dir)?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_result
    }

    fn recover_metadata<T: DeserializeOwned>(
        &self,
        kind: StateKind,
    ) -> Result<StateRecovery<T>, StoreError> {
        let dir = self.kind_dir(kind);
        create_private_dir(&dir)?;

        let mut records = Vec::new();
        let mut diagnostics = Vec::new();

        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }

            let id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown")
                .to_owned();

            match fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<T>(&content) {
                    Ok(metadata) => records.push(RecoveredMetadata { id, path, metadata }),
                    Err(error) => diagnostics.push(StateRecoveryDiagnostic {
                        path,
                        reason: format!("invalid {} metadata JSON: {error}", kind.label()),
                    }),
                },
                Err(error) => diagnostics.push(StateRecoveryDiagnostic {
                    path,
                    reason: format!("could not read {} metadata: {error}", kind.label()),
                }),
            }
        }

        Ok(StateRecovery {
            records,
            diagnostics,
        })
    }

    fn remove_metadata(&self, kind: StateKind, id: &str) -> Result<(), StoreError> {
        let path = self.metadata_path(kind, id);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::Io(error)),
        }
    }

    fn kind_dir(&self, kind: StateKind) -> PathBuf {
        self.state_dir.join(kind.dir_name())
    }

    fn metadata_path(&self, kind: StateKind, id: &str) -> PathBuf {
        self.kind_dir(kind)
            .join(format!("{}.json", safe_file_stem(id)))
    }

    fn temporary_path(&self, kind: StateKind, id: &str) -> PathBuf {
        let process = std::process::id();
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        self.kind_dir(kind).join(format!(
            ".{}.json.tmp.{process}.{counter}.{nanos}",
            safe_file_stem(id)
        ))
    }
}

/// Durable state files successfully recovered from one metadata directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateRecovery<T> {
    /// Valid metadata records.
    pub records: Vec<RecoveredMetadata<T>>,
    /// Invalid state files skipped during recovery.
    pub diagnostics: Vec<StateRecoveryDiagnostic>,
}

/// One recovered durable metadata record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredMetadata<T> {
    /// Redaction-safe ID derived from the metadata file name.
    pub id: String,
    /// Metadata path under `~/.cadis/state`.
    pub path: PathBuf,
    /// Parsed metadata payload.
    pub metadata: T,
}

/// Recovery diagnostic for a state file that failed safe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateRecoveryDiagnostic {
    /// Invalid metadata path under `~/.cadis/state`.
    pub path: PathBuf,
    /// Short parse or read error.
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StateKind {
    Session,
    Agent,
    AgentSession,
    Worker,
    Approval,
}

impl StateKind {
    fn all() -> [Self; 5] {
        [
            Self::Session,
            Self::Agent,
            Self::AgentSession,
            Self::Worker,
            Self::Approval,
        ]
    }

    fn dir_name(self) -> &'static str {
        match self {
            Self::Session => "sessions",
            Self::Agent => "agents",
            Self::AgentSession => "agent-sessions",
            Self::Worker => "workers",
            Self::Approval => "approvals",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Agent => "agent",
            Self::AgentSession => "AgentSession",
            Self::Worker => "worker",
            Self::Approval => "approval",
        }
    }
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

fn write_template_file_if_missing(path: &Path, content: &str) -> Result<(), StoreError> {
    if path.exists() {
        return Ok(());
    }

    atomic_write_private_file(path, content.as_bytes())
}

fn atomic_write_private_file(path: &Path, content: &[u8]) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state");
    let tmp_path = temporary_path_for(dir, file_name);

    let write_result = (|| -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        set_private_file_permissions(&tmp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        fs::rename(&tmp_path, path)?;
        sync_parent_dir(dir)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }

    write_result
}

fn temporary_path_for(dir: &Path, file_name: &str) -> PathBuf {
    let process = std::process::id();
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    dir.join(format!(
        ".{}.tmp.{process}.{counter}.{nanos}",
        safe_file_stem(file_name)
    ))
}

fn profile_toml_template(profile_id: &str) -> String {
    format!(
        r#"[profile]
id = "{profile_id}"
name = "{profile_id}"

[model]
default_provider = "auto"
default_model = "auto"

[security]
redact_logs = true
atomic_writes = true
deny_secret_paths = true
"#
    )
}

fn profile_gitignore_template() -> &'static str {
    ".env\nsecrets/\nchannels/\nsessions/\nworkers/\ncheckpoints/\nsandboxes/\nlogs/\nlocks/\nrun/\n*.key\n*.pem\n*.token\n*.sqlite-wal\n*.sqlite-shm\n"
}

fn agent_toml_template(template: &AgentHomeTemplate) -> Result<String, StoreError> {
    let metadata = AgentMetadataToml {
        agent: AgentIdentityToml {
            id: template.agent_id.to_string(),
            display_name: template.display_name.clone(),
            role: template.role.clone(),
            parent_agent_id: template.parent_agent_id.as_ref().map(ToString::to_string),
            model: template.model.clone(),
        },
        files: AgentFilesToml::default(),
    };
    let mut toml = redact(&toml::to_string_pretty(&metadata)?);
    if !toml.ends_with('\n') {
        toml.push('\n');
    }
    Ok(toml)
}

fn agent_policy_toml_template() -> Result<String, StoreError> {
    let mut toml = toml::to_string_pretty(&AgentPolicyToml::default())?;
    if !toml.ends_with('\n') {
        toml.push('\n');
    }
    Ok(toml)
}

fn skill_policy_toml_template() -> Result<String, StoreError> {
    let mut toml = toml::to_string_pretty(&SkillPolicyToml::default())?;
    if !toml.ends_with('\n') {
        toml.push('\n');
    }
    Ok(toml)
}

fn persona_template(template: &AgentHomeTemplate) -> String {
    format!(
        "# Persona\n\n{} is a CADIS agent with the {} role.\n",
        template.display_name, template.role
    )
}

fn instructions_template(template: &AgentHomeTemplate) -> String {
    format!(
        "# Instructions\n\nFollow CADIS daemon-owned policy and workspace grants for the {} role.\n",
        template.role
    )
}

fn user_template() -> &'static str {
    "# User\n\nProfile-specific user preferences may be summarized here.\n"
}

fn memory_template() -> &'static str {
    "# Memory\n\nDurable agent memory notes may be promoted here after review.\n"
}

fn tools_template() -> &'static str {
    "# Tools\n\nThis file is guidance only. Hard permissions belong in POLICY.toml and daemon policy.\n"
}

fn agent_readme_template() -> &'static str {
    "# Agent Home\n\nThis directory stores persistent agent identity, instructions, memory, skills, and policy metadata. It is not a project workspace.\n"
}

fn project_gitignore_template() -> &'static str {
    "worktrees/\nartifacts/\ntmp/\nlogs/\n*.key\n*.pem\n*.token\n.env\n"
}

fn path_diagnostic(name: &str, path: &Path, label: String) -> ProjectWorktreeDiagnostic {
    if path.is_dir() {
        ProjectWorktreeDiagnostic {
            name: name.to_owned(),
            status: "ok".to_owned(),
            message: format!("{label} exists at {}", path.display()),
        }
    } else {
        ProjectWorktreeDiagnostic {
            name: name.to_owned(),
            status: "warn".to_owned(),
            message: format!("{label} is stale or missing at {}", path.display()),
        }
    }
}

fn default_agent_denied_paths() -> Vec<PathBuf> {
    [
        "~/.ssh",
        "~/.aws",
        "~/.gnupg",
        "~/.config/gcloud",
        "~/.cadis/profiles/*/.env",
        "~/.cadis/profiles/*/secrets",
        "~/.cadis/profiles/*/channels/*/tokens",
        "/etc",
        "/dev",
        "/proc",
        "/sys",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

fn expand_home(path: &Path) -> PathBuf {
    let Some(value) = path.to_str() else {
        return path.to_path_buf();
    };

    if value == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(value));
    }

    if let Some(rest) = value.strip_prefix("~/") {
        return home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(value));
    }

    path.to_path_buf()
}

/// Returns the user home directory using platform-appropriate env vars.
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        env::var_os("HOME").map(PathBuf::from)
    }
}

fn create_private_dir(path: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(path)?;
    set_private_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

fn sync_parent_dir(path: &Path) -> Result<(), StoreError> {
    let dir = File::open(path)?;
    dir.sync_all()?;
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

fn safe_file_stem(value: &str) -> String {
    let component = safe_file_component(value);
    if component.is_empty() {
        "unnamed".to_owned()
    } else {
        component
    }
}

fn openai_api_key_from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Option<String> {
    ["CADIS_OPENAI_API_KEY", "OPENAI_API_KEY"]
        .into_iter()
        .find_map(|name| lookup(name).filter(|value| !value.trim().is_empty()))
}

// ---------------------------------------------------------------------------
// Track H: Denied paths enforcement
// ---------------------------------------------------------------------------

/// Denied paths loaded from config for mutating tool enforcement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeniedPaths {
    paths: Vec<PathBuf>,
}

impl DeniedPaths {
    /// Creates a denied-paths checker from config or defaults.
    pub fn from_config(_config: &CadisConfig) -> Self {
        Self::new(default_denied_paths_for_enforcement())
    }

    /// Creates a denied-paths checker from explicit paths.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().map(|p| expand_home(&p)).collect(),
        }
    }

    /// Returns the denied path list.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Returns `true` if the given path is denied.
    pub fn is_denied(&self, target: &Path) -> bool {
        cadis_policy::is_denied_path(target, &self.paths)
    }

    /// Checks a path and returns an error if denied.
    pub fn check(&self, target: &Path) -> Result<(), StoreError> {
        if self.is_denied(target) {
            Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("path {} is denied by policy", target.display()),
            )))
        } else {
            Ok(())
        }
    }
}

impl Default for DeniedPaths {
    fn default() -> Self {
        Self::new(default_denied_paths_for_enforcement())
    }
}

fn default_denied_paths_for_enforcement() -> Vec<PathBuf> {
    [
        "/etc",
        "/usr",
        "/boot",
        "/sys",
        "/proc",
        "~/.ssh",
        "~/.gnupg",
        "~/.cadis/state",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

// ---------------------------------------------------------------------------
// Track H: Worker worktree cleanup executor
// ---------------------------------------------------------------------------

/// Executes approved worker worktree cleanup by removing the directory.
pub struct WorktreeCleanupExecutor;

impl WorktreeCleanupExecutor {
    /// Removes a CADIS-owned worker worktree directory and updates project metadata.
    ///
    /// This is a daemon-internal privileged operation exempt from tool policy
    /// because it only operates on worktrees with CADIS-owned metadata records.
    pub fn execute(
        project_store: &ProjectWorkspaceStore,
        worker_id: &str,
    ) -> Result<(), StoreError> {
        let workspace_metadata = project_store.load()?;
        let paths = project_store.worker_worktree_paths(worker_id, workspace_metadata.as_ref());

        if paths.worktree_path.exists() {
            fs::remove_dir_all(&paths.worktree_path)?;
        }

        if let Some(mut metadata) = project_store.load_worker_worktree_metadata(worker_id)? {
            metadata.state = ProjectWorkerWorktreeState::Removed;
            project_store.save_worker_worktree_metadata(&metadata)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Track H: Media manifest
// ---------------------------------------------------------------------------

/// One entry in a project `.cadis/media/` manifest.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct MediaManifestEntry {
    /// Relative path inside `.cadis/media/`.
    pub path: PathBuf,
    /// Agent or tool that generated the file.
    pub generated_by: String,
    /// Tool name used for generation.
    pub tool: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Project-local media manifest stored as `.cadis/media/manifest.json`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct MediaManifest {
    /// Manifest entries.
    pub entries: Vec<MediaManifestEntry>,
}

/// Media manifest store rooted at a project workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaManifestStore {
    root: PathBuf,
}

impl MediaManifestStore {
    /// Creates a media manifest store for a project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            root: project_root.as_ref().join(".cadis").join("media"),
        }
    }

    /// Returns the manifest JSON path.
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// Loads the manifest, returning empty when missing.
    pub fn load(&self) -> Result<MediaManifest, StoreError> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(MediaManifest::default());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Saves the manifest with redaction.
    pub fn save(&self, manifest: &MediaManifest) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root)?;
        let mut json = redact(&serde_json::to_string_pretty(manifest)?);
        json.push('\n');
        atomic_write_private_file(&self.manifest_path(), json.as_bytes())
    }

    /// Appends one entry and saves.
    pub fn append(&self, mut entry: MediaManifestEntry) -> Result<(), StoreError> {
        const MANIFEST_FIELD_MAX: usize = 512;
        const MANIFEST_DESC_MAX: usize = 1024;
        entry.generated_by = entry
            .generated_by
            .chars()
            .take(MANIFEST_FIELD_MAX)
            .collect();
        entry.tool = entry.tool.chars().take(MANIFEST_FIELD_MAX).collect();
        if let Some(ref desc) = entry.description {
            entry.description = Some(desc.chars().take(MANIFEST_DESC_MAX).collect());
        }
        let mut manifest = self.load()?;
        manifest.entries.push(entry);
        self.save(&manifest)
    }
}

// ---------------------------------------------------------------------------
// Track H: Profile CRUD
// ---------------------------------------------------------------------------

impl CadisHome {
    /// Lists all profile IDs under `~/.cadis/profiles/`.
    pub fn list_profiles(&self) -> Result<Vec<String>, StoreError> {
        let profiles_dir = self.root.join("profiles");
        if !profiles_dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(profiles_dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    ids.push(name.to_owned());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Creates a new profile. Returns error if it already exists.
    pub fn create_profile(&self, profile_id: &str) -> Result<ProfileHome, StoreError> {
        let profile = self.profile(profile_id);
        if profile.root().exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("profile '{}' already exists", profile_id),
            )));
        }
        self.init_profile(profile_id)
    }

    /// Removes a profile directory. Returns error if it does not exist.
    pub fn remove_profile(&self, profile_id: &str) -> Result<(), StoreError> {
        let profile = self.profile(profile_id);
        if !profile.root().exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("profile '{}' does not exist", profile_id),
            )));
        }
        fs::remove_dir_all(profile.root())?;
        Ok(())
    }

    /// Exports a profile as a TOML string of its `profile.toml`.
    pub fn export_profile(&self, profile_id: &str) -> Result<String, StoreError> {
        let profile = self.profile(profile_id);
        if !profile.root().exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("profile '{}' does not exist", profile_id),
            )));
        }
        Ok(fs::read_to_string(profile.profile_config_path())?)
    }

    /// Imports a profile by writing `profile.toml` content into a new profile.
    pub fn import_profile(
        &self,
        profile_id: &str,
        content: &str,
    ) -> Result<ProfileHome, StoreError> {
        let profile = self.create_profile(profile_id)?;
        atomic_write_private_file(&profile.profile_config_path(), content.as_bytes())?;
        Ok(profile)
    }
}

// ---------------------------------------------------------------------------
// Track H: Checkpoint/rollback manager
// ---------------------------------------------------------------------------

/// Checkpoint manager that saves file copies before destructive operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointManager {
    checkpoints_dir: PathBuf,
}

/// One saved checkpoint.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Checkpoint {
    /// Unique checkpoint ID.
    pub id: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Reason for the checkpoint.
    pub reason: String,
    /// Files saved in this checkpoint (relative to workspace).
    pub files: Vec<String>,
}

impl CheckpointManager {
    /// Creates a checkpoint manager for a profile home.
    pub fn new(profile_home: &ProfileHome) -> Self {
        Self {
            checkpoints_dir: profile_home.root().join("checkpoints"),
        }
    }

    /// Returns the checkpoints root directory.
    pub fn checkpoints_dir(&self) -> &Path {
        &self.checkpoints_dir
    }

    /// Creates a checkpoint by copying target files.
    pub fn create(
        &self,
        checkpoint_id: &str,
        reason: &str,
        workspace: &Path,
        files: &[&Path],
    ) -> Result<Checkpoint, StoreError> {
        let checkpoint_dir = self.checkpoints_dir.join(safe_file_stem(checkpoint_id));
        create_private_dir(&checkpoint_dir)?;

        let mut saved_files = Vec::new();
        for file in files {
            let source = if file.is_absolute() {
                file.to_path_buf()
            } else {
                workspace.join(file)
            };
            if !source.is_file() {
                continue;
            }
            let relative = source
                .strip_prefix(workspace)
                .unwrap_or(&source)
                .to_string_lossy()
                .to_string();
            let relative = relative.strip_prefix('/').unwrap_or(&relative);
            let dest = checkpoint_dir.join("files").join(relative);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &dest)?;
            saved_files.push(relative.to_owned());
        }

        let checkpoint = Checkpoint {
            id: checkpoint_id.to_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
            reason: reason.to_owned(),
            files: saved_files,
        };
        let mut json = serde_json::to_string_pretty(&checkpoint)?;
        json.push('\n');
        atomic_write_private_file(&checkpoint_dir.join("checkpoint.json"), json.as_bytes())?;
        Ok(checkpoint)
    }

    /// Rolls back a checkpoint by restoring saved files.
    pub fn rollback(
        &self,
        checkpoint_id: &str,
        workspace: &Path,
    ) -> Result<Checkpoint, StoreError> {
        let checkpoint_dir = self.checkpoints_dir.join(safe_file_stem(checkpoint_id));
        let meta_path = checkpoint_dir.join("checkpoint.json");
        if !meta_path.exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("checkpoint '{}' does not exist", checkpoint_id),
            )));
        }
        let content = fs::read_to_string(&meta_path)?;
        let checkpoint: Checkpoint = serde_json::from_str(&content)?;

        for file in &checkpoint.files {
            // Prevent absolute paths from escaping the workspace.
            let relative = Path::new(file)
                .strip_prefix("/")
                .or_else(|_| Path::new(file).strip_prefix("."))
                .unwrap_or(Path::new(file));
            let saved = checkpoint_dir.join("files").join(relative);
            if saved.is_file() {
                let target = workspace.join(relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&saved, &target)?;
            }
        }

        Ok(checkpoint)
    }

    /// Lists checkpoint IDs.
    pub fn list(&self) -> Result<Vec<String>, StoreError> {
        if !self.checkpoints_dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.checkpoints_dir)? {
            let entry = entry?;
            if entry.path().is_dir() && entry.path().join("checkpoint.json").exists() {
                if let Some(name) = entry.file_name().to_str() {
                    ids.push(name.to_owned());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }
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
    fn parses_agent_spawn_limits() {
        let config = toml::from_str::<CadisConfig>(
            r#"
            [agent_spawn]
            max_depth = 1
            max_children_per_parent = 2
            max_total_agents = 16
            "#,
        )
        .expect("agent spawn config should parse");

        assert_eq!(config.agent_spawn.max_depth, 1);
        assert_eq!(config.agent_spawn.max_children_per_parent, 2);
        assert_eq!(config.agent_spawn.max_total_agents, 16);
        assert_eq!(
            config.ui_preferences()["agent_spawn"]["max_children_per_parent"],
            serde_json::json!(2)
        );
    }

    #[test]
    fn parses_agent_runtime_limits() {
        let config = toml::from_str::<CadisConfig>(
            r#"
            [agent_runtime]
            default_timeout_sec = 60
            max_steps_per_session = 3
            "#,
        )
        .expect("agent runtime config should parse");

        assert_eq!(config.agent_runtime.default_timeout_sec, 60);
        assert_eq!(config.agent_runtime.max_steps_per_session, 3);
        assert_eq!(
            config.ui_preferences()["agent_runtime"]["max_steps_per_session"],
            serde_json::json!(3)
        );
    }

    #[test]
    fn parses_daemon_owned_voice_config() {
        let config = toml::from_str::<CadisConfig>(
            r#"
            [voice]
            enabled = true
            provider = "system"
            voice_id = "en-US-AvaNeural"
            stt_language = "id"
            rate = 5
            pitch = -5
            volume = 10
            auto_speak = true
            max_spoken_chars = 500
            "#,
        )
        .expect("voice config should parse");

        assert!(config.voice.enabled);
        assert_eq!(config.voice.provider, "system");
        assert_eq!(config.voice.stt_language, "id");
        assert_eq!(config.voice.max_spoken_chars, 500);
        assert_eq!(
            config.ui_preferences()["voice"]["provider"],
            serde_json::json!("system")
        );
    }

    #[test]
    fn parses_orchestrator_config() {
        let config = toml::from_str::<CadisConfig>(
            r#"
            [orchestrator]
            worker_delegation_enabled = false
            default_worker_role = "Reviewer"
            "#,
        )
        .expect("orchestrator config should parse");

        assert!(!config.orchestrator.worker_delegation_enabled);
        assert_eq!(config.orchestrator.default_worker_role, "Reviewer");
        assert_eq!(
            config.ui_preferences()["orchestrator"]["default_worker_role"],
            serde_json::json!("Reviewer")
        );
    }

    #[test]
    fn effective_tcp_address_uses_configured_port_or_default() {
        let mut config = CadisConfig::default();
        assert_eq!(config.effective_tcp_address(), "127.0.0.1:7433");
        config.tcp_port = Some(9000);
        assert_eq!(config.effective_tcp_address(), "127.0.0.1:9000");
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

    #[test]
    fn approval_store_persists_redacted_records() {
        let config = test_config("approval-record");
        let store = ApprovalStore::new(&config.cadis_home);
        let record = ApprovalRecord {
            approval_id: ApprovalId::from("apr_1"),
            session_id: SessionId::from("ses_1"),
            tool_call_id: ToolCallId::from("tool_1"),
            tool_name: "shell.run".to_owned(),
            risk_class: RiskClass::SystemChange,
            title: "Approval needed".to_owned(),
            summary: "Run command".to_owned(),
            command: Some("OPENAI_API_KEY=sk-testsecretvalue123456".to_owned()),
            workspace: Some("/tmp/project".to_owned()),
            requested_at: Timestamp::new_utc("2026-04-26T00:00:00Z")
                .expect("timestamp should parse"),
            expires_at: Timestamp::new_utc("2026-04-26T00:05:00Z").expect("timestamp should parse"),
            state: ApprovalState::Pending,
            decision: None,
            reason: None,
            resolved_at: None,
        };

        store.save(&record).expect("record should save");

        let loaded = store
            .load(&ApprovalId::from("apr_1"))
            .expect("record should load")
            .expect("record should exist");
        assert_eq!(loaded.command.as_deref(), Some("OPENAI_API_KEY=[REDACTED]"));
    }

    #[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
    struct TestMetadata {
        id: String,
        status: String,
        api_key: Option<String>,
    }

    fn test_config(name: &str) -> CadisConfig {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should be after Unix epoch")
            .as_nanos();
        CadisConfig {
            cadis_home: env::temp_dir()
                .join(format!("cadis-store-{name}-{}-{nanos}", std::process::id())),
            ..CadisConfig::default()
        }
    }

    #[test]
    fn state_store_uses_redaction_safe_state_paths() {
        let config = test_config("paths");
        ensure_layout(&config).expect("layout should be created");
        let store = StateStore::new(&config);

        assert_eq!(
            store.session_path(&SessionId::from("ses/../1")),
            config.cadis_home.join("state/sessions/ses____1.json")
        );
        assert!(config.cadis_home.join("state/agents").is_dir());
        assert!(config.cadis_home.join("state/agent-sessions").is_dir());
        assert!(config.cadis_home.join("state/workers").is_dir());
        assert!(config.cadis_home.join("state/approvals").is_dir());
    }

    #[test]
    fn atomic_state_write_round_trips_and_redacts_secret_values() {
        let config = test_config("write");
        let store = StateStore::new(&config);
        let agent_id = AgentId::from("agent/main");
        let metadata = TestMetadata {
            id: "agent/main".to_owned(),
            status: "ready".to_owned(),
            api_key: Some("sk-testsecretvalue123456".to_owned()),
        };

        store
            .write_agent_metadata(&agent_id, &metadata)
            .expect("agent metadata should write");

        let raw =
            fs::read_to_string(store.agent_path(&agent_id)).expect("metadata should be readable");
        assert!(raw.contains("[REDACTED]"));
        assert!(!raw.contains("sk-testsecretvalue123456"));

        let recovered = store
            .recover_agent_metadata::<TestMetadata>()
            .expect("agent metadata should recover");
        assert_eq!(recovered.diagnostics, Vec::new());
        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].id, "agent_main");
        assert_eq!(recovered.records[0].metadata.status, "ready");
        assert_eq!(
            recovered.records[0].metadata.api_key.as_deref(),
            Some("[REDACTED]")
        );
    }

    #[test]
    fn recovery_skips_corrupt_json_and_ignores_partial_temp_files() {
        let config = test_config("recover");
        let store = StateStore::new(&config);
        let session_id = SessionId::from("ses_1");
        let metadata = TestMetadata {
            id: "ses_1".to_owned(),
            status: "active".to_owned(),
            api_key: None,
        };

        store
            .write_session_metadata(&session_id, &metadata)
            .expect("session metadata should write");
        let sessions_dir = config.cadis_home.join("state/sessions");
        fs::write(sessions_dir.join("corrupt.json"), "{").expect("corrupt test state should write");
        fs::write(sessions_dir.join(".ses_2.json.tmp.1"), "{")
            .expect("partial temp state should write");

        let recovered = store
            .recover_session_metadata::<TestMetadata>()
            .expect("session recovery should fail safe");

        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].id, "ses_1");
        assert_eq!(recovered.records[0].metadata, metadata);
        assert_eq!(recovered.diagnostics.len(), 1);
        assert!(recovered.diagnostics[0]
            .path
            .ends_with("state/sessions/corrupt.json"));
        assert!(recovered.diagnostics[0]
            .reason
            .contains("invalid session metadata JSON"));
    }

    #[test]
    fn agent_session_metadata_round_trips_from_dedicated_state_dir() {
        let config = test_config("agent-session-write");
        let store = StateStore::new(&config);
        let agent_session_id = AgentSessionId::from("ags/../1");
        let metadata = TestMetadata {
            id: "ags/../1".to_owned(),
            status: "running".to_owned(),
            api_key: None,
        };

        store
            .write_agent_session_metadata(&agent_session_id, &metadata)
            .expect("AgentSession metadata should write");

        assert_eq!(
            store.agent_session_path(&agent_session_id),
            config.cadis_home.join("state/agent-sessions/ags____1.json")
        );

        let recovered = store
            .recover_agent_session_metadata::<TestMetadata>()
            .expect("AgentSession metadata should recover");
        assert_eq!(recovered.diagnostics, Vec::new());
        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].id, "ags____1");
        assert_eq!(recovered.records[0].metadata, metadata);
    }

    #[test]
    fn agent_session_recovery_skips_corrupt_json_and_partial_temp_files() {
        let config = test_config("agent-session-recover");
        let store = StateStore::new(&config);
        let agent_session_id = AgentSessionId::from("ags_1");
        let metadata = TestMetadata {
            id: "ags_1".to_owned(),
            status: "running".to_owned(),
            api_key: None,
        };

        store
            .write_agent_session_metadata(&agent_session_id, &metadata)
            .expect("AgentSession metadata should write");
        let agent_sessions_dir = config.cadis_home.join("state/agent-sessions");
        fs::write(agent_sessions_dir.join("corrupt.json"), "{")
            .expect("corrupt AgentSession state should write");
        fs::write(agent_sessions_dir.join(".ags_2.json.tmp.1"), "{")
            .expect("partial AgentSession temp state should write");

        let recovered = store
            .recover_agent_session_metadata::<TestMetadata>()
            .expect("AgentSession recovery should fail safe");

        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].id, "ags_1");
        assert_eq!(recovered.records[0].metadata, metadata);
        assert_eq!(recovered.diagnostics.len(), 1);
        assert!(recovered.diagnostics[0]
            .path
            .ends_with("state/agent-sessions/corrupt.json"));
        assert!(recovered.diagnostics[0]
            .reason
            .contains("invalid AgentSession metadata JSON"));
    }

    #[test]
    fn recovery_helpers_cover_worker_and_approval_metadata() {
        let config = test_config("worker-approval");
        let store = StateStore::new(&config);

        store
            .write_worker_metadata(
                "worker/1",
                &TestMetadata {
                    id: "worker/1".to_owned(),
                    status: "running".to_owned(),
                    api_key: None,
                },
            )
            .expect("worker metadata should write");
        store
            .write_approval_metadata(
                &ApprovalId::from("approval/1"),
                &TestMetadata {
                    id: "approval/1".to_owned(),
                    status: "pending".to_owned(),
                    api_key: None,
                },
            )
            .expect("approval metadata should write");

        let workers = store
            .recover_worker_metadata::<TestMetadata>()
            .expect("worker metadata should recover");
        let approvals = store
            .recover_approval_metadata::<TestMetadata>()
            .expect("approval metadata should recover");

        assert_eq!(workers.records[0].id, "worker_1");
        assert_eq!(workers.records[0].metadata.status, "running");
        assert_eq!(approvals.records[0].id, "approval_1");
        assert_eq!(approvals.records[0].metadata.status, "pending");
    }

    #[test]
    fn profile_layout_initializes_templates_and_preserves_legacy_paths() {
        let config = test_config("profile-layout");
        ensure_layout(&config).expect("layout should initialize");

        let cadis_home = CadisHome::new(&config.cadis_home);
        let profile = cadis_home.profile("default");

        assert!(config.cadis_home.join("state/sessions").is_dir());
        assert!(config.cadis_home.join("sessions").is_dir());
        assert!(profile.root().is_dir());
        assert!(profile
            .agent_home("agent/../main")
            .ends_with("agents/agent____main"));
        assert!(profile.workspaces_dir().is_dir());
        assert!(profile.root().join("eventlog").is_dir());

        let profile_toml = fs::read_to_string(profile.profile_config_path())
            .expect("profile template should be readable");
        assert!(profile_toml.contains("id = \"default\""));

        let gitignore =
            fs::read_to_string(profile.gitignore_path()).expect(".gitignore should be readable");
        assert!(gitignore.contains(".env"));
        assert!(gitignore.contains("secrets/"));

        let registry =
            fs::read_to_string(profile.workspace_registry_path()).expect("registry should exist");
        assert_eq!(registry, "workspace = []\n");
    }

    #[test]
    fn agent_home_initializes_typed_templates() {
        let config = test_config("agent-home");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");
        let template = AgentHomeTemplate::new(
            AgentId::from("coder/main"),
            "Coder",
            "Coding",
            Some(AgentId::from("main")),
            "echo",
        );

        let agent = profile
            .init_agent(&template)
            .expect("agent home should initialize");

        assert!(agent.root().ends_with("agents/coder_main"));
        assert!(agent.root().join("PERSONA.md").is_file());
        assert!(agent.root().join("INSTRUCTIONS.md").is_file());
        assert!(agent.root().join("MEMORY.md").is_file());
        assert!(agent.root().join("TOOLS.md").is_file());
        assert!(agent.root().join("SKILL_POLICY.toml").is_file());
        assert!(agent.root().join("memory/daily").is_dir());
        assert!(agent.root().join("memory/decisions.md").is_file());

        let metadata = agent.load_metadata().expect("AGENT.toml should parse");
        assert_eq!(metadata.agent.id, "coder/main");
        assert_eq!(metadata.agent.display_name, "Coder");
        assert_eq!(metadata.agent.role, "Coding");
        assert_eq!(metadata.agent.parent_agent_id.as_deref(), Some("main"));
        assert_eq!(metadata.files.policy, PathBuf::from("POLICY.toml"));

        let policy = agent.load_policy().expect("POLICY.toml should parse");
        assert_eq!(policy.policy.version, 1);
        assert_eq!(
            policy.policy.default_workspace_access,
            vec![WorkspaceAccess::Read]
        );
        assert!(policy
            .sandbox
            .denied_paths
            .contains(&PathBuf::from("~/.ssh")));
        assert!(policy.policy.approval_required);
    }

    #[test]
    fn agent_doctor_reports_corrupt_and_oversized_files() {
        let config = test_config("agent-doctor");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");
        let agent = profile
            .init_agent(&AgentHomeTemplate::new(
                AgentId::from("main"),
                "CADIS",
                "Orchestrator",
                None,
                "echo",
            ))
            .expect("agent home should initialize");

        fs::write(agent.policy_toml_path(), "[policy\n").expect("corrupt policy should write");
        fs::write(agent.root().join("PERSONA.md"), "x".repeat(128))
            .expect("oversized persona should write");

        let diagnostics = profile
            .agent_doctor_diagnostics(AgentHomeDoctorOptions {
                max_agent_toml_bytes: 1024,
                max_policy_toml_bytes: 1024,
                max_text_file_bytes: 16,
                max_memory_file_bytes: 1024,
            })
            .expect("agent doctor should run");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "main/agent.POLICY.toml" && diagnostic.status == "error"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "main/agent.PERSONA.md" && diagnostic.status == "warn"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "profile.agents" && diagnostic.message == "1 agent home(s) found"
        }));
    }

    #[test]
    fn workspace_registry_round_trips_toml_and_expands_home() {
        let config = test_config("workspace-registry");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");
        let store = profile.workspace_registry();
        let registry = WorkspaceRegistry {
            workspace: vec![WorkspaceMetadata {
                id: "example-project".to_owned(),
                kind: WorkspaceKind::Project,
                root: PathBuf::from("~/Project/example"),
                vcs: WorkspaceVcs::Git,
                owner: Some("rama".to_owned()),
                trusted: true,
                worktree_root: Some(PathBuf::from(".cadis/worktrees")),
                artifact_root: Some(PathBuf::from(".cadis/artifacts")),
                checkpoint_policy: CheckpointPolicy::Enabled,
                aliases: vec![WorkspaceAlias {
                    workspace_id: "example-project".to_owned(),
                    aliases: vec!["example".to_owned(), "demo".to_owned()],
                }],
            }],
        };

        store.save(&registry).expect("registry should save");
        let raw = fs::read_to_string(store.path()).expect("registry should be readable");
        assert!(raw.contains("[[workspace]]"));
        assert!(raw.contains("[[workspace.alias]]"));

        let loaded = store.load().expect("registry should load");
        assert_eq!(loaded.workspace.len(), 1);
        assert_eq!(loaded.workspace[0].id, "example-project");
        assert_eq!(loaded.workspace[0].kind, WorkspaceKind::Project);
        assert_eq!(
            loaded.workspace[0].root,
            expand_home(Path::new("~/Project/example"))
        );
        assert_eq!(
            loaded.workspace[0].aliases[0].aliases,
            vec!["example", "demo"]
        );
    }

    #[test]
    fn project_workspace_metadata_round_trips_and_initializes_layout() {
        let config = test_config("project-workspace-metadata");
        let root = config.cadis_home.join("project");
        fs::create_dir_all(&root).expect("project root should be created");
        let store = ProjectWorkspaceStore::new(&root);
        let metadata = ProjectWorkspaceMetadata {
            workspace_id: "example-project".to_owned(),
            kind: WorkspaceKind::Project,
            vcs: WorkspaceVcs::Git,
            worktree_root: PathBuf::from(".cadis/worktrees"),
            artifact_root: PathBuf::from(".cadis/artifacts"),
            media_root: PathBuf::from(".cadis/media"),
        };

        assert_eq!(store.load().expect("missing metadata should load"), None);
        store.save(&metadata).expect("project metadata should save");

        assert!(store.workspace_toml_path().is_file());
        assert!(root.join(".cadis/worktrees").is_dir());
        assert!(root.join(".cadis/artifacts").is_dir());
        assert!(root.join(".cadis/media").is_dir());
        assert!(root.join(".cadis/.gitignore").is_file());

        let loaded = store
            .load()
            .expect("project metadata should load")
            .expect("project metadata should exist");
        assert_eq!(loaded, metadata);
    }

    #[test]
    fn worker_artifact_paths_are_profile_scoped() {
        let config = test_config("worker-artifacts");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");

        let paths = profile
            .ensure_worker_artifact_layout("worker/1")
            .expect("worker artifact layout should initialize");

        assert_eq!(
            paths.root,
            profile.root().join("artifacts/workers/worker_1")
        );
        assert_eq!(paths.patch, paths.root.join("patch.diff"));
        assert!(paths.root.is_dir());
    }

    #[test]
    fn project_worker_worktree_metadata_round_trips_and_reports_stale_paths() {
        let config = test_config("project-worker-worktree");
        let root = config.cadis_home.join("project");
        fs::create_dir_all(&root).expect("project root should be created");
        let store = ProjectWorkspaceStore::new(&root);
        store
            .save(&ProjectWorkspaceMetadata {
                workspace_id: "example-project".to_owned(),
                kind: WorkspaceKind::Project,
                vcs: WorkspaceVcs::Git,
                worktree_root: PathBuf::from(".cadis/worktrees"),
                artifact_root: PathBuf::from(".cadis/artifacts"),
                media_root: PathBuf::from(".cadis/media"),
            })
            .expect("project metadata should save");

        let paths = store.worker_worktree_paths("worker/1", store.load().unwrap().as_ref());
        let metadata = ProjectWorkerWorktreeMetadata {
            worker_id: "worker/1".to_owned(),
            workspace_id: "example-project".to_owned(),
            worktree_path: PathBuf::from(".cadis/worktrees/worker_1"),
            branch_name: "cadis/worker_1/example".to_owned(),
            base_ref: Some("HEAD".to_owned()),
            state: ProjectWorkerWorktreeState::Planned,
            artifact_root: config
                .cadis_home
                .join("profiles/default/artifacts/workers/worker_1"),
        };

        store
            .save_worker_worktree_metadata(&metadata)
            .expect("worker worktree metadata should save");

        assert!(paths.metadata_path.is_file());
        let loaded = store
            .load_worker_worktree_metadata("worker/1")
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(loaded, metadata);

        let diagnostics = store
            .worker_worktree_diagnostics()
            .expect("worker worktree diagnostics should run");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "workspace.worktrees.worker_1.path" && diagnostic.status == "warn"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "workspace.worktrees.worker_1.artifacts"
                && diagnostic.status == "warn"
        }));

        fs::create_dir_all(root.join(".cadis/worktrees/worker_1"))
            .expect("worktree dir should be created");
        fs::create_dir_all(
            config
                .cadis_home
                .join("profiles/default/artifacts/workers/worker_1"),
        )
        .expect("artifact root should be created");
        let diagnostics = store
            .worker_worktree_diagnostics()
            .expect("worker worktree diagnostics should run");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "workspace.worktrees.worker_1.path" && diagnostic.status == "ok"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.name == "workspace.worktrees.worker_1.artifacts" && diagnostic.status == "ok"
        }));
    }

    #[test]
    fn workspace_grants_are_append_only_recoverable_and_redacted() {
        let config = test_config("workspace-grants");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");
        let store = profile.workspace_grants();
        let record = WorkspaceGrantRecord {
            grant_id: "grant/1".to_owned(),
            profile_id: "default".to_owned(),
            agent_id: Some(AgentId::from("main")),
            workspace_id: "example-project".to_owned(),
            root: PathBuf::from("/tmp/example-project"),
            access: vec![WorkspaceAccess::Read, WorkspaceAccess::Write],
            created_at: Timestamp::new_utc("2026-04-26T00:00:00Z").expect("timestamp should parse"),
            expires_at: Some(
                Timestamp::new_utc("2026-04-26T01:00:00Z").expect("timestamp should parse"),
            ),
            source: GrantSource::User,
            reason: Some("OPENAI_API_KEY=sk-testsecretvalue123456".to_owned()),
        };

        store.append(&record).expect("grant should append");

        let raw = fs::read_to_string(store.path()).expect("grant log should be readable");
        assert!(raw.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(!raw.contains("sk-testsecretvalue123456"));

        let recovered = store.load().expect("grant log should load");
        assert_eq!(recovered.diagnostics, Vec::new());
        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].grant_id, "grant/1");
        assert_eq!(
            recovered.records[0].reason.as_deref(),
            Some("OPENAI_API_KEY=[REDACTED]")
        );
    }

    #[test]
    fn workspace_grant_recovery_reports_invalid_jsonl_lines() {
        let config = test_config("workspace-grant-recovery");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");
        let store = profile.workspace_grants();

        fs::write(store.path(), "{\n").expect("invalid grant line should write");

        let recovered = store.load().expect("grant recovery should fail safe");
        assert_eq!(recovered.records, Vec::new());
        assert_eq!(recovered.diagnostics.len(), 1);
        assert_eq!(recovered.diagnostics[0].line, 1);
        assert!(recovered.diagnostics[0]
            .reason
            .contains("invalid workspace grant JSON"));
    }

    #[test]
    fn denied_paths_blocks_system_paths() {
        let denied = DeniedPaths::default();
        assert!(denied.is_denied(Path::new("/etc/passwd")));
        assert!(denied.is_denied(Path::new("/usr/bin/ls")));
        assert!(denied.is_denied(Path::new("/boot/vmlinuz")));
        assert!(denied.is_denied(Path::new("/sys/class")));
        assert!(denied.is_denied(Path::new("/proc/1")));
        assert!(!denied.is_denied(Path::new("/tmp/safe")));
    }

    #[test]
    fn denied_paths_check_returns_error_for_denied() {
        let denied = DeniedPaths::default();
        assert!(denied.check(Path::new("/etc")).is_err());
        assert!(denied.check(Path::new("/tmp")).is_ok());
    }

    #[test]
    fn media_manifest_round_trips() {
        let config = test_config("media-manifest");
        let root = config.cadis_home.join("project");
        fs::create_dir_all(&root).expect("project root should be created");
        let store = MediaManifestStore::new(&root);

        assert!(store.load().unwrap().entries.is_empty());

        store
            .append(MediaManifestEntry {
                path: PathBuf::from("image.png"),
                generated_by: "agent/main".to_owned(),
                tool: "image.generate".to_owned(),
                created_at: "2026-04-28T00:00:00Z".to_owned(),
                description: Some("test image".to_owned()),
            })
            .expect("entry should append");

        let manifest = store.load().expect("manifest should load");
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].generated_by, "agent/main");
    }

    #[test]
    fn profile_crud_create_list_export_import_remove() {
        let config = test_config("profile-crud");
        let home = CadisHome::new(&config.cadis_home);
        home.init_profile("default").expect("default should init");

        home.create_profile("test-profile")
            .expect("profile should be created");
        assert!(home.create_profile("test-profile").is_err());

        let profiles = home.list_profiles().expect("profiles should list");
        assert!(profiles.contains(&"test-profile".to_owned()));

        let exported = home
            .export_profile("test-profile")
            .expect("profile should export");
        assert!(exported.contains("test-profile"));

        home.import_profile("imported", &exported)
            .expect("profile should import");
        let profiles = home.list_profiles().expect("profiles should list");
        assert!(profiles.contains(&"imported".to_owned()));

        home.remove_profile("test-profile")
            .expect("profile should be removed");
        assert!(home.remove_profile("test-profile").is_err());
    }

    #[test]
    fn media_manifest_truncates_long_fields() {
        let config = test_config("media-manifest-truncate");
        let root = config.cadis_home.join("project");
        fs::create_dir_all(&root).expect("project root should be created");
        let store = MediaManifestStore::new(&root);
        let long = "x".repeat(1000);
        let long_desc = "x".repeat(2000);
        store
            .append(MediaManifestEntry {
                path: PathBuf::from("test.png"),
                generated_by: long.clone(),
                tool: long,
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                description: Some(long_desc),
            })
            .unwrap();
        let manifest = store.load().unwrap();
        assert_eq!(manifest.entries[0].generated_by.len(), 512);
        assert_eq!(manifest.entries[0].tool.len(), 512);
        assert_eq!(
            manifest.entries[0].description.as_ref().unwrap().len(),
            1024
        );
    }

    #[test]
    fn checkpoint_create_rollback_list() {
        let config = test_config("checkpoint");
        let profile = CadisHome::new(&config.cadis_home)
            .init_profile("default")
            .expect("profile should initialize");
        let manager = CheckpointManager::new(&profile);
        let workspace = config.cadis_home.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        let file = workspace.join("test.txt");
        fs::write(&file, "original").expect("file should write");

        let checkpoint = manager
            .create("cp_1", "before edit", &workspace, &[Path::new("test.txt")])
            .expect("checkpoint should be created");
        assert_eq!(checkpoint.files, vec!["test.txt"]);

        fs::write(&file, "modified").expect("file should be modified");
        assert_eq!(fs::read_to_string(&file).unwrap(), "modified");

        manager
            .rollback("cp_1", &workspace)
            .expect("rollback should succeed");
        assert_eq!(fs::read_to_string(&file).unwrap(), "original");

        let ids = manager.list().expect("list should succeed");
        assert!(ids.contains(&"cp_1".to_owned()));
    }
}
