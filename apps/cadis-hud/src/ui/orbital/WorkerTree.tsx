/**
 * Collapsible worker tree shown under each agent widget.
 *
 * Lists workers whose `parentAgentId` matches the agent's id. Workers without
 * an explicit parent are also listed under any agent whose id appears as the
 * worker's `cli` (so e.g. "codex" workers attach to the Codex agent).
 */
import { useState } from "react";
import {
  useHud,
  selectAgentSessions,
  selectWorkers,
  type AgentSessionRecord,
  type WorkerRecord,
} from "../hudState.js";

const STATUS_COLOR: Record<WorkerRecord["status"], string> = {
  spawning: "var(--warn)",
  running: "var(--ok)",
  completed: "var(--accent)",
  failed: "var(--err)",
  cancelled: "var(--faint)",
};

const SESSION_STATUS_COLOR: Record<AgentSessionRecord["status"], string> = {
  started: "var(--warn)",
  running: "var(--ok)",
  completed: "var(--accent)",
  failed: "var(--err)",
  cancelled: "var(--faint)",
  timed_out: "var(--err)",
  budget_exceeded: "var(--err)",
};

export type WorkerTreeProps = {
  agentId: string;
  /** Initial collapsed state. Defaults to expanded when there are workers. */
  defaultOpen?: boolean;
};

function workerBelongsTo(w: WorkerRecord, agentId: string): boolean {
  if (w.parentAgentId === agentId) return true;
  if (w.agentId === agentId) return true;
  if (!w.parentAgentId && w.cli && w.cli.toLowerCase() === agentId.toLowerCase()) return true;
  return false;
}

function sessionBelongsTo(s: AgentSessionRecord, agentId: string): boolean {
  return s.agentId === agentId || s.parentAgentId === agentId;
}

export function WorkerTree({ agentId, defaultOpen }: WorkerTreeProps) {
  const all = useHud(selectWorkers);
  const allSessions = useHud(selectAgentSessions);
  const selectedWorkerId = useHud((s) => s.selectedWorkerId);
  const selectWorker = useHud((s) => s.selectWorker);
  const workers = all.filter((w) => workerBelongsTo(w, agentId));
  const sessions = allSessions.filter((s) => sessionBelongsTo(s, agentId));
  const [open, setOpen] = useState(defaultOpen ?? true);

  if (workers.length === 0 && sessions.length === 0) return null;

  const runningWorkers = workers.filter((w) => w.status === "running" || w.status === "spawning").length;

  return (
    <div className="worker-tree" data-agent={agentId}>
      <button
        type="button"
        className="worker-tree__toggle"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="worker-tree__caret">{open ? "▾" : "▸"}</span>
        <span className="worker-tree__label">
          workers · {workers.length}
          {runningWorkers > 0 ? ` · ${runningWorkers} active` : ""}
        </span>
      </button>
      {open && (
        <ul className="worker-tree__list">
          {sessions.map((s) => (
            <li key={s.id} className="worker-tree__item worker-tree__item--session" data-status={s.status}>
              <span
                className="worker-tree__dot"
                style={{ background: SESSION_STATUS_COLOR[s.status] }}
                aria-hidden
              />
              <span className="worker-tree__id" title={s.id}>{compactId(s.id)}</span>
              <span className="worker-tree__status">{s.status.replace("_", " ")}</span>
              <span className="worker-tree__text" title={s.task}>{sessionText(s)}</span>
              <ProgressBar value={sessionProgress(s)} />
            </li>
          ))}
          {workers.map((w) => (
            <li key={w.id} className="worker-tree__row">
              <button
                type="button"
                className="worker-tree__item worker-tree__item--button"
                data-status={w.status}
                data-selected={selectedWorkerId === w.id ? "true" : undefined}
                onClick={() => selectWorker(w.id)}
                aria-label={`Open code work for ${w.id}`}
                aria-current={selectedWorkerId === w.id ? "true" : undefined}
              >
                <span
                  className="worker-tree__dot"
                  style={{ background: STATUS_COLOR[w.status] }}
                  aria-hidden
                />
                <span className="worker-tree__id" title={w.id}>{compactId(w.id)}</span>
                <span className="worker-tree__status">{w.status}</span>
                <span className="worker-tree__text" title={workerTitle(w)}>
                  {workerText(w)}
                </span>
                <ProgressBar value={workerProgress(w)} indeterminate={w.status === "running"} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function ProgressBar({
  value,
  indeterminate = false,
}: {
  value: number;
  indeterminate?: boolean;
}) {
  return (
    <span
      className={`worker-tree__progress${indeterminate ? " worker-tree__progress--active" : ""}`}
      aria-hidden
    >
      <span style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
    </span>
  );
}

function sessionProgress(session: AgentSessionRecord): number {
  if (session.status === "completed") return 100;
  if (session.status === "failed" || session.status === "cancelled") return 100;
  if (session.status === "timed_out" || session.status === "budget_exceeded") return 100;
  if (session.budgetSteps <= 0) return session.status === "started" ? 8 : 18;
  return Math.max(8, Math.round((session.stepsUsed / session.budgetSteps) * 100));
}

function workerProgress(worker: WorkerRecord): number {
  if (worker.status === "completed" || worker.status === "failed" || worker.status === "cancelled") {
    return 100;
  }
  if (worker.status === "spawning") return 12;
  return Math.min(92, 30 + worker.logLineCount * 12);
}

function sessionText(session: AgentSessionRecord): string {
  const progress =
    session.budgetSteps > 0
      ? `step ${Math.min(session.stepsUsed, session.budgetSteps)}/${session.budgetSteps}`
      : session.status.replace("_", " ");
  return `${session.task} · ${progress}`;
}

function workerText(worker: WorkerRecord): string {
  const worktree = worker.worktree?.state ? `worktree ${worker.worktree.state}` : undefined;
  return worker.lastText ?? worker.summary ?? worktree ?? worker.cli ?? "daemon worker";
}

function workerTitle(worker: WorkerRecord): string {
  const parts = [
    worker.lastText ?? worker.summary,
    worker.worktree?.branchName,
    worker.worktree?.worktreePath,
  ].filter(Boolean);
  return parts.join(" · ");
}

function compactId(id: string): string {
  return id.length > 16 ? `${id.slice(0, 13)}...` : id;
}
