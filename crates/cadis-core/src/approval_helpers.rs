//! Approval-related helper functions extracted from lib.rs.

use cadis_protocol::ApprovalRequestPayload;
use cadis_store::ApprovalRecord;
use chrono::{DateTime, Utc};

/// Checks if an approval record has expired.
pub fn approval_is_expired(record: &ApprovalRecord) -> bool {
    DateTime::parse_from_rfc3339(record.expires_at.as_str())
        .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

/// Converts an approval record to an approval request payload.
pub fn approval_request_payload(record: &ApprovalRecord) -> ApprovalRequestPayload {
    ApprovalRequestPayload {
        approval_id: record.approval_id.clone(),
        session_id: record.session_id.clone(),
        tool_call_id: record.tool_call_id.clone(),
        risk_class: record.risk_class,
        title: record.title.clone(),
        summary: record.summary.clone(),
        command: record.command.clone(),
        workspace: record.workspace.clone(),
        expires_at: record.expires_at.clone(),
    }
}
