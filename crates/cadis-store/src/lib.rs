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
    AgentId, ApprovalDecision, ApprovalId, EventEnvelope, RiskClass, SessionId, Timestamp,
    ToolCallId,
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
    /// Model provider settings.
    pub model: ModelConfig,
    /// HUD settings.
    pub hud: HudConfig,
    /// Voice settings.
    pub voice: VoiceConfig,
    /// Request-driven agent spawn limits.
    pub agent_spawn: AgentSpawnConfig,
    /// Daemon-owned orchestrator settings.
    pub orchestrator: OrchestratorConfig,
    /// Profile-home selection and profile layout settings.
    pub profile: ProfileConfig,
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
            agent_spawn: AgentSpawnConfig::default(),
            orchestrator: OrchestratorConfig::default(),
            profile: ProfileConfig::default(),
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
            "orchestrator": {
                "worker_delegation_enabled": self.orchestrator.worker_delegation_enabled,
                "default_worker_role": self.orchestrator.default_worker_role
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

    /// Creates a workspace registry helper for this profile.
    pub fn workspace_registry(&self) -> WorkspaceRegistryStore {
        WorkspaceRegistryStore::new(self.clone())
    }

    /// Creates a workspace grant helper for this profile.
    pub fn workspace_grants(&self) -> WorkspaceGrantStore {
        WorkspaceGrantStore::new(self.clone())
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
    Worker,
    Approval,
}

impl StateKind {
    fn all() -> [Self; 4] {
        [Self::Session, Self::Agent, Self::Worker, Self::Approval]
    }

    fn dir_name(self) -> &'static str {
        match self {
            Self::Session => "sessions",
            Self::Agent => "agents",
            Self::Worker => "workers",
            Self::Approval => "approvals",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Agent => "agent",
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
}
