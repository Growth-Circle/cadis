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
}
