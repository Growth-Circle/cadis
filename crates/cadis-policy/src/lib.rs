//! Central approval and risk policy primitives for CADIS.

use cadis_protocol::RiskClass;
use serde::{Deserialize, Serialize};

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

/// Default policy engine.
#[derive(Clone, Debug, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    /// Decides the default behavior for a risk class.
    pub fn decide(&self, risk_class: RiskClass) -> PolicyDecision {
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
            PolicyEngine.decide(RiskClass::SafeRead),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn risky_operations_require_approval() {
        assert_eq!(
            PolicyEngine.decide(RiskClass::SudoSystem),
            PolicyDecision::RequireApproval
        );
    }

    #[test]
    fn safe_read_tools_are_allowed() {
        let decision = PolicyEngine.decide_tool("file.read");

        assert_eq!(decision.risk_class, Some(RiskClass::SafeRead));
        assert_eq!(decision.decision, PolicyDecision::Allow);
    }

    #[test]
    fn shell_requires_approval() {
        let decision = PolicyEngine.decide_tool("shell.run");

        assert_eq!(decision.risk_class, Some(RiskClass::SystemChange));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn unknown_tool_is_denied() {
        let decision = PolicyEngine.decide_tool("browser.open");

        assert_eq!(decision.risk_class, None);
        assert_eq!(decision.decision, PolicyDecision::Deny);
    }

    #[test]
    fn secret_access_classification_requires_approval() {
        let decision =
            PolicyEngine.decide_classification(classify_secret_access(SecretAccessSource::Config));

        assert_eq!(decision.risk_class, Some(RiskClass::SecretAccess));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn dangerous_delete_classification_requires_approval() {
        let decision = PolicyEngine.decide_classification(classify_dangerous_delete(
            WorkspacePathScope::InsideWorkspace,
        ));

        assert_eq!(decision.risk_class, Some(RiskClass::DangerousDelete));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn outside_workspace_write_classification_requires_approval() {
        let decision = PolicyEngine.decide_classification(classify_workspace_mutation(
            WorkspacePathScope::OutsideWorkspace,
        ));

        assert_eq!(decision.risk_class, Some(RiskClass::OutsideWorkspace));
        assert_eq!(decision.decision, PolicyDecision::RequireApproval);
    }

    #[test]
    fn unclassified_workspace_mutation_is_denied() {
        let decision = PolicyEngine.decide_action(PolicyAction::WorkspaceMutation {
            target_scope: WorkspacePathScope::Unknown,
        });

        assert_eq!(decision.risk_class, None);
        assert_eq!(decision.decision, PolicyDecision::Deny);
    }
}
