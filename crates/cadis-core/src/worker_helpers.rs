//! Worker-related helper functions extracted from lib.rs.

use crate::{
    truncate_redacted_text, WorkerCommandFailure, WorkerCommandReport, WorkerRecord,
    WORKER_COMMAND_SUMMARY_LIMIT_BYTES,
};
use cadis_protocol::{
    WorkerEventPayload, WorkerState, WorkerWorktreeCleanupPolicy, WorkerWorktreeState,
};
use cadis_store::{ProjectWorkerWorktreeState, WorkerArtifactPathSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerLifecycleEventKind {
    Started,
    Completed,
    Failed,
    Cancelled,
}

/// Returns true if the worker state is terminal.
pub fn worker_status_is_terminal(status: WorkerState) -> bool {
    status.is_terminal()
}

/// Converts a worker state to its string representation.
pub fn worker_state_str(s: WorkerState) -> String {
    match s {
        WorkerState::Queued => "queued",
        WorkerState::Running => "running",
        WorkerState::Completed => "completed",
        WorkerState::Failed => "failed",
        WorkerState::Cancelled => "cancelled",
    }
    .to_owned()
}

/// Converts a worker event payload to the appropriate lifecycle event.
pub fn worker_lifecycle_event(payload: WorkerEventPayload) -> cadis_protocol::CadisEvent {
    let kind = payload.status.as_deref().and_then(|s| match s {
        "completed" => Some(WorkerLifecycleEventKind::Completed),
        "cancelled" | "canceled" => Some(WorkerLifecycleEventKind::Cancelled),
        "failed" => Some(WorkerLifecycleEventKind::Failed),
        "running" | "queued" => Some(WorkerLifecycleEventKind::Started),
        _ => None,
    });
    match kind {
        Some(WorkerLifecycleEventKind::Completed) => {
            cadis_protocol::CadisEvent::WorkerCompleted(payload)
        }
        Some(WorkerLifecycleEventKind::Failed) => cadis_protocol::CadisEvent::WorkerFailed(payload),
        Some(WorkerLifecycleEventKind::Cancelled) => {
            cadis_protocol::CadisEvent::WorkerCancelled(payload)
        }
        Some(WorkerLifecycleEventKind::Started) | None => {
            cadis_protocol::CadisEvent::WorkerStarted(payload)
        }
    }
}

/// Determines the lifecycle event kind for a worker state.
pub fn worker_lifecycle_event_kind(status: WorkerState) -> WorkerLifecycleEventKind {
    match status {
        WorkerState::Completed => WorkerLifecycleEventKind::Completed,
        WorkerState::Cancelled => WorkerLifecycleEventKind::Cancelled,
        status if worker_status_is_terminal(status) => WorkerLifecycleEventKind::Failed,
        _ => WorkerLifecycleEventKind::Started,
    }
}

/// Determines the terminal worktree state for a worker.
pub fn worker_terminal_worktree_state(
    record: &WorkerRecord,
    status: WorkerState,
) -> Option<WorkerWorktreeState> {
    if !worker_status_is_terminal(status) {
        return None;
    }
    let worktree = record.worktree.as_ref()?;
    if worktree.state != WorkerWorktreeState::Active {
        return None;
    }
    if worker_lifecycle_event_kind(status) == WorkerLifecycleEventKind::Cancelled {
        return Some(WorkerWorktreeState::CleanupPending);
    }

    Some(match worktree.cleanup_policy {
        WorkerWorktreeCleanupPolicy::OnCompletion => WorkerWorktreeState::CleanupPending,
        WorkerWorktreeCleanupPolicy::Explicit | WorkerWorktreeCleanupPolicy::AfterApply => {
            WorkerWorktreeState::ReviewPending
        }
    })
}

/// Plans the terminal worktree state for a worker record.
pub fn plan_worker_terminal_worktree(record: &mut WorkerRecord, status: WorkerState) {
    let Some(state) = worker_terminal_worktree_state(record, status) else {
        return;
    };
    if let Some(worktree) = &mut record.worktree {
        worktree.state = state;
    }
}

/// Projects a worker worktree state for a given worker state.
pub fn project_worker_worktree_state_for_worker_state(
    state: WorkerWorktreeState,
) -> ProjectWorkerWorktreeState {
    match state {
        WorkerWorktreeState::Planned => ProjectWorkerWorktreeState::Planned,
        WorkerWorktreeState::Active => ProjectWorkerWorktreeState::Ready,
        WorkerWorktreeState::ReviewPending => ProjectWorkerWorktreeState::ReviewPending,
        WorkerWorktreeState::CleanupPending => ProjectWorkerWorktreeState::CleanupPending,
        WorkerWorktreeState::Removed => ProjectWorkerWorktreeState::Removed,
    }
}

/// Converts worker artifact paths to protocol artifact locations.
pub fn worker_artifact_locations(
    paths: &WorkerArtifactPathSet,
) -> cadis_protocol::WorkerArtifactLocations {
    cadis_protocol::WorkerArtifactLocations {
        root: paths.root.display().to_string(),
        patch: paths.patch.display().to_string(),
        test_report: paths.test_report.display().to_string(),
        summary: paths.summary.display().to_string(),
        changed_files: paths.changed_files.display().to_string(),
        memory_candidates: paths.memory_candidates.display().to_string(),
    }
}

/// Builds the command report JSON from a worker command report.
pub fn worker_command_report_json(report: &WorkerCommandReport) -> serde_json::Value {
    serde_json::json!({
        "command": report.command,
        "cwd": report.cwd,
        "exit_code": report.exit_code,
        "stdout": report.stdout,
        "stderr": report.stderr,
        "timed_out": report.timed_out,
        "timeout_ms": report.timeout_ms,
    })
}

/// Extracts command failure details from a worker command report.
pub fn worker_command_failure(report: &WorkerCommandReport) -> WorkerCommandFailure {
    if report.timed_out {
        return WorkerCommandFailure {
            code: "worker_command_timeout".to_owned(),
            message: format!(
                "worker command timed out after timeout_ms={}: {}",
                report.timeout_ms, report.command
            ),
        };
    }

    let detail = if !report.stderr.trim().is_empty() {
        report.stderr.trim()
    } else if !report.stdout.trim().is_empty() {
        report.stdout.trim()
    } else {
        "command exited without output"
    };
    WorkerCommandFailure {
        code: "worker_command_failed".to_owned(),
        message: format!(
            "worker command exited with code {:?}: {}",
            report.exit_code,
            truncate_redacted_text(detail, WORKER_COMMAND_SUMMARY_LIMIT_BYTES)
        ),
    }
}

/// Extracts command logs from a worker command report.
pub fn worker_command_logs(report: &WorkerCommandReport) -> Vec<String> {
    let mut logs = Vec::new();
    if !report.stdout.is_empty() {
        logs.push(format!("STDOUT:\n{}", report.stdout));
    }
    if !report.stderr.is_empty() {
        logs.push(format!("STDERR:\n{}", report.stderr));
    }
    logs
}

/// Generates a markdown summary from a worker command report.
pub fn worker_command_summary_markdown(report: &WorkerCommandReport) -> String {
    let mut summary = String::new();
    summary.push_str(&format!("**Command**: `{}`\n\n", report.command));
    summary.push_str(&format!(
        "**Exit Code**: {}\n\n",
        report
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "None".to_string())
    ));
    if report.timed_out {
        summary.push_str("**Status**: Timed out\n\n");
    }
    if !report.stdout.is_empty() {
        summary.push_str(&format!("**STDOUT**:\n```\n{}\n```\n\n", report.stdout));
    }
    if !report.stderr.is_empty() {
        summary.push_str(&format!("**STDERR**:\n```\n{}\n```\n\n", report.stderr));
    }
    summary
}
