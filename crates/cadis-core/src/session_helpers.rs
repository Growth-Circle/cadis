//! Session-related helper functions extracted from lib.rs.

use cadis_protocol::{AgentSessionEventPayload, AgentSessionStatus, CadisEvent};

/// Converts an agent session event payload to the appropriate lifecycle event.
pub fn agent_session_lifecycle_event(payload: AgentSessionEventPayload) -> CadisEvent {
    match payload.status {
        AgentSessionStatus::Completed => CadisEvent::AgentSessionCompleted(payload),
        AgentSessionStatus::Failed
        | AgentSessionStatus::TimedOut
        | AgentSessionStatus::BudgetExceeded => CadisEvent::AgentSessionFailed(payload),
        AgentSessionStatus::Cancelled => CadisEvent::AgentSessionCancelled(payload),
        AgentSessionStatus::Started | AgentSessionStatus::Running => {
            CadisEvent::AgentSessionUpdated(payload)
        }
    }
}

/// Returns a human-readable label for an agent session status.
pub fn agent_session_status_label(status: AgentSessionStatus) -> &'static str {
    match status {
        AgentSessionStatus::Started => "started",
        AgentSessionStatus::Running => "running",
        AgentSessionStatus::Completed => "completed",
        AgentSessionStatus::Failed => "failed",
        AgentSessionStatus::TimedOut => "timed_out",
        AgentSessionStatus::BudgetExceeded => "budget_exceeded",
        AgentSessionStatus::Cancelled => "cancelled",
    }
}

/// Returns true if the agent session status is terminal.
pub fn agent_session_is_terminal(status: AgentSessionStatus) -> bool {
    matches!(
        status,
        AgentSessionStatus::Completed
            | AgentSessionStatus::Failed
            | AgentSessionStatus::TimedOut
            | AgentSessionStatus::BudgetExceeded
            | AgentSessionStatus::Cancelled
    )
}
