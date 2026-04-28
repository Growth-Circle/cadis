//! Central approval and risk policy primitives for CADIS.

use cadis_protocol::RiskClass;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Policy decision returned before an operation may run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Operation may run without explicit approval.
    Allow,
    /// Operation must create an approval request first.
    RequireApproval,
    /// Operation must not run.
    Deny,
}

// ── Tool trait (Track D item 1) ──────────────────────────────────────

/// Trait for daemon-registered tools.
pub trait Tool: Send + Sync {
    /// Stable tool name (e.g. `file.read`).
    fn name(&self) -> &str;
    /// Risk class for policy decisions.
    fn risk_class(&self) -> RiskClass;
    /// Whether the tool requires approval before execution.
    fn requires_approval(&self) -> bool;
}

// ── Policy config from TOML (Track D item 2) ────────────────────────

/// Loadable policy configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct PolicyConfig {
    /// Per-risk-class overrides.
    pub risk_overrides: Vec<RiskOverride>,
    /// Denied path prefixes checked before any tool execution.
    pub denied_paths: Vec<PathBuf>,
    /// Secret file patterns that require explicit policy allow.
    pub secret_patterns: Vec<String>,
    /// Shell environment variable allowlist.
    pub shell_env_allowlist: Vec<String>,
    /// Whether secret access is allowed when explicitly granted.
    pub allow_explicit_secret_access: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            risk_overrides: Vec::new(),
            denied_paths: Vec::new(),
            secret_patterns: default_secret_patterns(),
            shell_env_allowlist: default_shell_env_allowlist(),
            allow_explicit_secret_access: false,
        }
    }
}

impl PolicyConfig {
    /// Loads policy config from a TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Loads policy config from a TOML file, falling back to defaults.
    pub fn from_file(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| Self::from_toml(&content).ok())
            .unwrap_or_default()
    }
}

/// Per-risk-class policy override.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RiskOverride {
    /// Risk class name (matches `RiskClass` variant names in snake_case).
    pub risk_class: String,
    /// Decision override.
    pub decision: String,
}

fn default_secret_patterns() -> Vec<String> {
    [
        ".env",
        ".env.*",
        "*.pem",
        "*.key",
        "*.p12",
        "*.pfx",
        "credentials",
        "id_rsa",
        "id_ed25519",
        "id_ecdsa",
        "id_dsa",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".git-credentials",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

fn default_shell_env_allowlist() -> Vec<String> {
    [
        "PATH",
        "HOME",
        "USER",
        "LANG",
        "TERM",
        "SHELL",
        "LC_ALL",
        "LC_CTYPE",
        "TMPDIR",
        "PWD",
        "CADIS_WORKER_ID",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

// ── Cancellation token (Track D item 4) ──────────────────────────────

/// Typed cancellation token for cooperative tool cancellation.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    /// Creates a new uncancelled token.
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

// ── Denied path enforcement (Track D item 6) ────────────────────────

/// Checks whether a path is denied by the configured denied path list.
pub fn is_denied_path(path: &Path, denied_paths: &[PathBuf]) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        // If we can't resolve, check raw prefix match.
        return denied_paths.iter().any(|denied| path.starts_with(denied));
    };
    denied_paths.iter().any(|denied| {
        if let Ok(denied_canonical) = denied.canonicalize() {
            canonical.starts_with(&denied_canonical)
        } else {
            canonical.starts_with(denied)
        }
    })
}

// ── Secret access gating (Track D item 7) ────────────────────────────

/// Returns true if the file name matches a secret-bearing pattern.
pub fn is_secret_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || matches!(
            lower.as_str(),
            ".netrc"
                | ".npmrc"
                | ".pypirc"
                | ".git-credentials"
                | "credentials"
                | "id_rsa"
                | "id_dsa"
                | "id_ecdsa"
                | "id_ed25519"
        )
        || lower.contains("secret")
        || lower.contains("credential")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("private_key")
}

// ── Shell env filtering (Track D item 3) ─────────────────────────────

/// Filters the current process environment to only allowed variables.
pub fn filtered_env(allowlist: &[String]) -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| allowlist.iter().any(|allowed| allowed == key))
        .collect()
}

/// Returns the default shell environment allowlist.
pub fn shell_env_allowlist() -> Vec<String> {
    default_shell_env_allowlist()
}

// ── Policy engine ────────────────────────────────────────────────────

/// Default policy engine.
#[derive(Clone, Debug, Default)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    /// Creates a policy engine with the given config.
    pub fn with_config(config: PolicyConfig) -> Self {
        Self { config }
    }

    /// Returns the active policy config.
    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    /// Decides the default behavior for a risk class.
    pub fn decide(&self, risk_class: RiskClass) -> PolicyDecision {
        // Check overrides first.
        let risk_name = format!("{risk_class:?}");
        for ov in &self.config.risk_overrides {
            if ov.risk_class.eq_ignore_ascii_case(&risk_name) {
                return match ov.decision.as_str() {
                    "allow" => PolicyDecision::Allow,
                    "deny" => PolicyDecision::Deny,
                    _ => PolicyDecision::RequireApproval,
                };
            }
        }
        match risk_class {
            RiskClass::SafeRead => PolicyDecision::Allow,
            RiskClass::WorkspaceEdit
            | RiskClass::NetworkAccess
            | RiskClass::SecretAccess
            | RiskClass::SystemChange
            | RiskClass::DangerousDelete
            | RiskClass::OutsideWorkspace
            | RiskClass::GitPushMain
            | RiskClass::GitForcePush
            | RiskClass::SudoSystem => PolicyDecision::RequireApproval,
        }
    }

    /// Checks whether a path is denied by policy.
    pub fn is_path_denied(&self, path: &Path) -> bool {
        is_denied_path(path, &self.config.denied_paths)
    }

    /// Checks whether a file is secret-bearing and access is not allowed.
    pub fn is_secret_access_denied(&self, path: &Path) -> bool {
        is_secret_file(path) && !self.config.allow_explicit_secret_access
    }

    /// Returns the filtered environment for shell execution.
    pub fn shell_env(&self) -> Vec<(String, String)> {
        filtered_env(&self.config.shell_env_allowlist)
    }

    /// Decides the default behavior for a structured risk classification.
    pub fn decide_classification(
        &self,
        classification: PolicyClassification,
    ) -> ClassifiedPolicyDecision {
        let decision = match classification.risk_class {
            Some(risk_class) => self.decide(risk_class),
            None => PolicyDecision::Deny,
        };

        ClassifiedPolicyDecision {
            risk_class: classification.risk_class,
            decision,
            reason: classification.reason,
        }
    }

    /// Classifies and decides a structured policy action.
    pub fn decide_action(&self, action: PolicyAction) -> ClassifiedPolicyDecision {
        self.decide_classification(classify_action(action))
    }

    /// Classifies and decides a tool call by stable tool name.
    pub fn decide_tool(&self, tool_name: &str) -> ToolPolicyDecision {
        let Some(risk_class) = classify_tool(tool_name) else {
            return ToolPolicyDecision {
                risk_class: None,
                decision: PolicyDecision::Deny,
                reason: "unknown tool is denied by default".to_owned(),
            };
        };

        ToolPolicyDecision {
            risk_class: Some(risk_class),
            decision: self.decide(risk_class),
            reason: tool_policy_reason(tool_name, risk_class),
        }
    }
}

/// Policy classification for an action before execution.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PolicyClassification {
    /// Classified risk class, absent when classification failed closed.
    pub risk_class: Option<RiskClass>,
    /// Redacted reason suitable for events and logs.
    pub reason: String,
}

impl PolicyClassification {
    fn classified(risk_class: RiskClass, reason: impl Into<String>) -> Self {
        Self {
            risk_class: Some(risk_class),
            reason: reason.into(),
        }
    }

    fn unclassified(reason: impl Into<String>) -> Self {
        Self {
            risk_class: None,
            reason: reason.into(),
        }
    }
}

/// Policy decision with structured risk metadata.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ClassifiedPolicyDecision {
    /// Classified risk class, absent when the action could not be classified.
    pub risk_class: Option<RiskClass>,
    /// Final policy decision.
    pub decision: PolicyDecision,
    /// Redacted reason suitable for events and logs.
    pub reason: String,
}

/// Policy decision with tool-specific risk metadata.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ToolPolicyDecision {
    /// Classified risk class, absent when the tool is unknown.
    pub risk_class: Option<RiskClass>,
    /// Final policy decision.
    pub decision: PolicyDecision,
    /// Redacted reason suitable for events and logs.
    pub reason: String,
}

/// Path scope resolved before workspace mutation classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePathScope {
    /// Target was resolved inside an approved workspace.
    InsideWorkspace,
    /// Target was resolved outside the approved workspace boundary.
    OutsideWorkspace,
    /// Target scope is unavailable or could not be resolved.
    Unknown,
}

/// Source category for secret access classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretAccessSource {
    /// Environment variable or process environment secret.
    Environment,
    /// Config file or profile setting that may contain a secret.
    Config,
    /// File path known or suspected to contain credentials.
    File,
    /// OS keychain, credential helper, or future secret store.
    CredentialStore,
    /// Secret source is known to exist but its category is not available.
    Unknown,
}

/// Risk hints supplied by shell command validation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct ShellExecutionRisk {
    /// Command uses sudo or equivalent privilege escalation.
    pub uses_sudo: bool,
    /// Command may read environment, config, or credential secrets.
    pub reads_secrets: bool,
    /// Command performs recursive or otherwise destructive deletion.
    pub dangerous_delete: bool,
    /// Command mutates system packages, services, devices, or global state.
    pub mutates_system: bool,
}

/// Structured action shape for policy classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    /// File or workspace mutation after path scope validation.
    WorkspaceMutation {
        /// Resolved target scope.
        target_scope: WorkspacePathScope,
    },
    /// Shell command execution after command validation.
    ShellExecution {
        /// Shell risk hints.
        risk: ShellExecutionRisk,
    },
    /// Secret read or credential lookup.
    SecretAccess {
        /// Secret source category.
        source: SecretAccessSource,
    },
    /// Recursive or destructive deletion.
    DangerousDelete {
        /// Resolved target scope.
        target_scope: WorkspacePathScope,
    },
}

/// Classifies a structured policy action.
pub fn classify_action(action: PolicyAction) -> PolicyClassification {
    match action {
        PolicyAction::WorkspaceMutation { target_scope } => {
            classify_workspace_mutation(target_scope)
        }
        PolicyAction::ShellExecution { risk } => classify_shell_execution(risk),
        PolicyAction::SecretAccess { source } => classify_secret_access(source),
        PolicyAction::DangerousDelete { target_scope } => classify_dangerous_delete(target_scope),
    }
}

/// Classifies a workspace mutation after path scope validation.
pub fn classify_workspace_mutation(target_scope: WorkspacePathScope) -> PolicyClassification {
    match target_scope {
        WorkspacePathScope::InsideWorkspace => PolicyClassification::classified(
            RiskClass::WorkspaceEdit,
            "workspace mutation inside approved workspace requires approval",
        ),
        WorkspacePathScope::OutsideWorkspace => PolicyClassification::classified(
            RiskClass::OutsideWorkspace,
            "workspace mutation targets a path outside the approved workspace",
        ),
        WorkspacePathScope::Unknown => PolicyClassification::unclassified(
            "workspace mutation target scope is unknown and is denied by default",
        ),
    }
}

/// Classifies shell execution from structured command risk hints.
pub fn classify_shell_execution(risk: ShellExecutionRisk) -> PolicyClassification {
    if risk.uses_sudo {
        return PolicyClassification::classified(
            RiskClass::SudoSystem,
            "shell execution uses privilege escalation",
        );
    }

    if risk.dangerous_delete {
        return PolicyClassification::classified(
            RiskClass::DangerousDelete,
            "shell execution performs dangerous deletion",
        );
    }

    if risk.reads_secrets {
        return PolicyClassification::classified(
            RiskClass::SecretAccess,
            "shell execution may access secrets",
        );
    }

    if risk.mutates_system {
        return PolicyClassification::classified(
            RiskClass::SystemChange,
            "shell execution mutates system state",
        );
    }

    PolicyClassification::classified(
        RiskClass::SystemChange,
        "shell execution requires policy approval",
    )
}

/// Classifies explicit secret access.
pub fn classify_secret_access(source: SecretAccessSource) -> PolicyClassification {
    let reason = match source {
        SecretAccessSource::Environment => "environment secret access requires approval",
        SecretAccessSource::Config => "config secret access requires approval",
        SecretAccessSource::File => "secret file access requires approval",
        SecretAccessSource::CredentialStore => "credential store access requires approval",
        SecretAccessSource::Unknown => "secret access requires approval",
    };

    PolicyClassification::classified(RiskClass::SecretAccess, reason)
}

/// Classifies recursive or otherwise destructive deletion.
pub fn classify_dangerous_delete(target_scope: WorkspacePathScope) -> PolicyClassification {
    let reason = match target_scope {
        WorkspacePathScope::InsideWorkspace => {
            "dangerous delete inside approved workspace requires approval"
        }
        WorkspacePathScope::OutsideWorkspace => {
            "dangerous delete outside approved workspace requires approval"
        }
        WorkspacePathScope::Unknown => {
            "dangerous delete target scope is unknown and requires approval"
        }
    };

    PolicyClassification::classified(RiskClass::DangerousDelete, reason)
}

fn classify_tool(tool_name: &str) -> Option<RiskClass> {
    match tool_name {
        "file.read" | "file.search" | "git.status" => Some(RiskClass::SafeRead),
        "file.write"
        | "file.patch"
        | "git.diff"
        | "git.worktree.create"
        | "git.worktree.remove" => Some(RiskClass::WorkspaceEdit),
        "shell.run" => Some(RiskClass::SystemChange),
        _ => None,
    }
}

fn tool_policy_reason(tool_name: &str, risk_class: RiskClass) -> String {
    match risk_class {
        RiskClass::SafeRead => format!("{tool_name} is a read-only tool"),
        _ => format!("{tool_name} requires approval for {risk_class:?} risk"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_read_is_allowed() {
        assert_eq!(
            PolicyEngine::default().decide(RiskClass::SafeRead),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn risky_operations_require_approval() {
        assert_eq!(
            PolicyEngine::default().decide(RiskClass::SudoSystem),
            PolicyDecision::RequireApproval
        );
    }

    #[test]
    fn safe_read_tools_are_allowed() {
        let decision = PolicyEngine::default().decide_tool("file.read");

        assert_eq!(decision.risk_class, Some(RiskClass::SafeRead));
        assert_eq!(decision.decision, PolicyDecision::Allow);
    }

    #[test]
    fn shell_requires_approval() {
        let decision = PolicyEngine::default().decide_tool("shell.run");

        assert_eq!(decision.risk_class, Some(RiskClass::SystemChange));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn unknown_tool_is_denied() {
        let decision = PolicyEngine::default().decide_tool("browser.open");

        assert_eq!(decision.risk_class, None);
        assert_eq!(decision.decision, PolicyDecision::Deny);
    }

    #[test]
    fn secret_access_classification_requires_approval() {
        let decision = PolicyEngine::default()
            .decide_classification(classify_secret_access(SecretAccessSource::Config));

        assert_eq!(decision.risk_class, Some(RiskClass::SecretAccess));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn dangerous_delete_classification_requires_approval() {
        let decision = PolicyEngine::default().decide_classification(classify_dangerous_delete(
            WorkspacePathScope::InsideWorkspace,
        ));

        assert_eq!(decision.risk_class, Some(RiskClass::DangerousDelete));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn outside_workspace_write_classification_requires_approval() {
        let decision = PolicyEngine::default().decide_classification(classify_workspace_mutation(
            WorkspacePathScope::OutsideWorkspace,
        ));

        assert_eq!(decision.risk_class, Some(RiskClass::OutsideWorkspace));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn unclassified_workspace_mutation_is_denied() {
        let decision = PolicyEngine::default().decide_action(PolicyAction::WorkspaceMutation {
            target_scope: WorkspacePathScope::Unknown,
        });

        assert_eq!(decision.risk_class, None);
        assert_eq!(decision.decision, PolicyDecision::Deny);
    }

    // ── Track D: Policy config from TOML ─────────────────────────────

    #[test]
    fn policy_config_loads_from_toml() {
        let toml = r#"
allow_explicit_secret_access = true
denied_paths = ["/etc/shadow"]
shell_env_allowlist = ["PATH", "HOME"]

[[risk_overrides]]
risk_class = "SafeRead"
decision = "deny"
"#;
        let config = PolicyConfig::from_toml(toml).expect("valid TOML");
        assert!(config.allow_explicit_secret_access);
        assert_eq!(config.denied_paths, vec![PathBuf::from("/etc/shadow")]);
        assert_eq!(config.shell_env_allowlist, vec!["PATH", "HOME"]);
        assert_eq!(config.risk_overrides.len(), 1);
    }

    #[test]
    fn policy_config_risk_override_changes_decision() {
        let config = PolicyConfig {
            risk_overrides: vec![RiskOverride {
                risk_class: "SafeRead".to_owned(),
                decision: "deny".to_owned(),
            }],
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::with_config(config);
        assert_eq!(engine.decide(RiskClass::SafeRead), PolicyDecision::Deny);
    }

    #[test]
    fn policy_config_defaults_are_sensible() {
        let config = PolicyConfig::default();
        assert!(!config.allow_explicit_secret_access);
        assert!(config.shell_env_allowlist.contains(&"PATH".to_owned()));
        assert!(config.shell_env_allowlist.contains(&"HOME".to_owned()));
        assert!(!config.secret_patterns.is_empty());
    }

    // ── Track D: Cancellation token ──────────────────────────────────

    #[test]
    fn cancellation_token_starts_uncancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancellation_token_becomes_cancelled() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
    }

    // ── Track D: Secret file detection ───────────────────────────────

    #[test]
    fn secret_file_detection() {
        assert!(is_secret_file(Path::new(".env")));
        assert!(is_secret_file(Path::new(".env.production")));
        assert!(is_secret_file(Path::new("server.pem")));
        assert!(is_secret_file(Path::new("private.key")));
        assert!(is_secret_file(Path::new("id_rsa")));
        assert!(is_secret_file(Path::new("credentials")));
        assert!(is_secret_file(Path::new("my_api_key.txt")));
        assert!(!is_secret_file(Path::new("README.md")));
        assert!(!is_secret_file(Path::new("main.rs")));
    }

    // ── Track D: Shell env filtering ─────────────────────────────────

    #[test]
    fn filtered_env_only_includes_allowlist() {
        let result = filtered_env(&["PATH".to_owned()]);
        assert!(result.iter().all(|(key, _)| key == "PATH"));
    }

    #[test]
    fn shell_env_allowlist_has_expected_entries() {
        let list = shell_env_allowlist();
        assert!(list.contains(&"PATH".to_owned()));
        assert!(list.contains(&"HOME".to_owned()));
        assert!(list.contains(&"USER".to_owned()));
        assert!(list.contains(&"LANG".to_owned()));
        assert!(list.contains(&"TERM".to_owned()));
        assert!(list.contains(&"SHELL".to_owned()));
    }

    // ── Track D: Denied path enforcement ─────────────────────────────

    #[test]
    fn denied_path_blocks_matching_prefix() {
        let denied = vec![PathBuf::from("/etc")];
        assert!(is_denied_path(Path::new("/etc/shadow"), &denied));
        assert!(!is_denied_path(Path::new("/tmp/safe"), &denied));
    }
}
