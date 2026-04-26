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
}
