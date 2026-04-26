/**
 * Collapsible worker tree shown under each agent widget.
 *
 * Lists workers whose `parentAgentId` matches the agent's id. Workers without
 * an explicit parent are also listed under any agent whose id appears as the
 * worker's `cli` (so e.g. "codex" workers attach to the Codex agent).
 */
import { useState } from "react";
import { useHud, selectWorkers, type WorkerRecord } from "../hudState.js";

const STATUS_COLOR: Record<WorkerRecord["status"], string> = {
  spawning: "var(--warn)",
  running: "var(--ok)",
  completed: "var(--accent)",
  failed: "var(--err)",
  cancelled: "var(--faint)",
};

export type WorkerTreeProps = {
  agentId: string;
  /** Initial collapsed state. Defaults to expanded when there are workers. */
  defaultOpen?: boolean;
};

function workerBelongsTo(w: WorkerRecord, agentId: string): boolean {
  if (w.parentAgentId === agentId) return true;
  if (!w.parentAgentId && w.cli && w.cli.toLowerCase() === agentId.toLowerCase()) return true;
  return false;
}

export function WorkerTree({ agentId, defaultOpen }: WorkerTreeProps) {
  const all = useHud(selectWorkers);
  const workers = all.filter((w) => workerBelongsTo(w, agentId));
  const [open, setOpen] = useState(defaultOpen ?? true);

  if (workers.length === 0) return null;

  return (
    <div className="worker-tree" data-agent={agentId}>
      <button
        type="button"
        className="worker-tree__toggle"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="worker-tree__caret">{open ? "▾" : "▸"}</span>
        <span className="worker-tree__label">workers · {workers.length}</span>
      </button>
      {open && (
        <ul className="worker-tree__list">
          {workers.map((w) => (
            <li key={w.id} className="worker-tree__item" data-status={w.status}>
              <span
                className="worker-tree__dot"
                style={{ background: STATUS_COLOR[w.status] }}
                aria-hidden
              />
              <span className="worker-tree__id">{w.id}</span>
              <span className="worker-tree__status">{w.status}</span>
              {w.lastText && <span className="worker-tree__text">{w.lastText}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
