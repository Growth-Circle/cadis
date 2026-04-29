import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useHud, type WorkerRecord } from "../hudState.js";
import { sendWorkerCleanup } from "../cadisActions.js";

const EMPTY_VALUE = "not reported by daemon";

export function CodeWorkPanel() {
  const open = useHud((s) => s.codeWorkPanelOpen);
  const selectedWorkerId = useHud((s) => s.selectedWorkerId);
  const workers = useHud((s) => s.workers);
  const agents = useHud((s) => s.agents);
  const setOpen = useHud((s) => s.setCodeWorkPanelOpen);

  const worker = selectedWorkerId
    ? workers.find((candidate) => candidate.id === selectedWorkerId)
    : null;
  const owner = worker
    ? agents.find((agent) => agent.spec.id === (worker.agentId ?? worker.parentAgentId))
    : null;
  const patches = useHud((s) => s.patches);
  const testResults = useHud((s) => s.testResults);
  const logLines = useMemo(() => recentLogLines(worker?.logTail ?? []), [worker?.logTail]);
  const isTerminal = worker
    ? ["completed", "failed", "cancelled"].includes(worker.status)
    : false;

  if (!open) return null;

  return (
    <aside className="code-work-panel" aria-label="Code work">
      <header className="code-work-panel__header">
        <div>
          <span className="code-work-panel__eyebrow">CODE WORK</span>
          <h2>{worker ? compactId(worker.id) : "No worker selected"}</h2>
        </div>
        <button
          type="button"
          className="code-work-panel__close"
          onClick={() => setOpen(false)}
          aria-label="Close code work panel"
        >
          X
        </button>
      </header>

      {worker ? (
        <>
          <div className="code-work-panel__statusline">
            <span className={`code-work-panel__status code-work-panel__status--${statusTone(worker.status)}`}>
              {worker.status}
            </span>
            <span>{owner?.spec.name ?? worker.agentId ?? worker.parentAgentId ?? "daemon worker"}</span>
          </div>

          <section className="code-work-panel__section" aria-labelledby="code-work-summary">
            <h3 id="code-work-summary">Summary</h3>
            <p className="code-work-panel__summary">{workerSummary(worker)}</p>
          </section>

          <section className="code-work-panel__section" aria-labelledby="code-work-artifacts">
            <h3 id="code-work-artifacts">Daemon Artifacts</h3>
            <dl className="code-work-panel__fields">
              <Field label="Worktree path" value={worker.worktree?.worktreePath} />
              <Field label="Worker cwd" value={worker.cwd} />
              <Field label="Diff/Patch" value={worker.artifacts?.patch} />
              <Field label="Changed files" value={worker.artifacts?.changedFiles} />
              <Field
                label="Test report"
                value={worker.artifacts?.testReport}
                status={worker.artifacts?.testReportStatus}
              />
            </dl>
          </section>

          {/* TODO: Inline diff content requires a future daemon `worker.artifact.read` request.
             worker.artifacts.patch is a file PATH, not diff content, so DiffViewer/FileTree
             cannot render useful output until the daemon provides actual content. */}

          {patches.length > 0 && (
            <section className="code-work-panel__section" aria-labelledby="code-work-patches">
              <h3 id="code-work-patches">Patches</h3>
              <ul className="code-work-panel__log">
                {patches.map((p) => (
                  <li key={p.id}><code>{p.summary}</code></li>
                ))}
              </ul>
            </section>
          )}

          {testResults.length > 0 && (
            <section className="code-work-panel__section" aria-labelledby="code-work-tests">
              <h3 id="code-work-tests">Test Results</h3>
              <ul className="code-work-panel__log">
                {testResults.map((t) => (
                  <li key={t.id}><code>{t.summary}</code></li>
                ))}
              </ul>
            </section>
          )}

          <section className="code-work-panel__section" aria-labelledby="code-work-log">
            <h3 id="code-work-log">Recent Log Tail</h3>
            {logLines.length ? (
              <ol className="code-work-panel__log">
                {logLines.map((line, index) => (
                  <li key={`${index}-${line}`}>
                    <code>{line}</code>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="code-work-panel__empty">No daemon log tail published.</p>
            )}
          </section>
        </>
      ) : (
        <p className="code-work-panel__empty">
          Select a daemon worker from the worker tree to inspect its worktree, artifacts, and log tail.
        </p>
      )}

      <footer className="code-work-panel__actions">
        <div>
          <button
            type="button"
            disabled
            title="Pending daemon worker.apply support"
          >
            APPLY
          </button>
          <button
            type="button"
            disabled={!worker || !isTerminal}
            title="Send worker.cleanup to daemon"
            onClick={() => worker && sendWorkerCleanup(worker.id, worker.worktree?.worktreePath)}
          >
            DISCARD
          </button>
          <button
            type="button"
            disabled={!worker?.worktree?.worktreePath}
            title="Open worktree in external editor"
            onClick={() => worker?.worktree?.worktreePath && openInEditor(worker.worktree.worktreePath)}
          >
            OPEN IN EDITOR
          </button>
        </div>
      </footer>
    </aside>
  );
}

/* ── Item 2: Inline diff viewer ─────────────────────────────────── */
/* TODO [BLOCKED]: Inline diff viewer requires daemon `worker.artifact.read` request
   (not yet implemented). worker.artifacts.patch is a file PATH, not diff content.
   Restore DiffViewer when daemon can return actual diff content. */

/* ── Item 3: Changed-files list & file tree ─────────────────────── */
/* TODO [BLOCKED]: Full changed-files content and file tree require daemon
   `worker.artifact.read` to read worktree contents. The artifact path reference
   is shown; inline content is blocked on daemon support. */

/* ── Item 5: Open in editor (Tauri command) ─────────────────────── */

function openInEditor(worktreePath: string): void {
  void invoke("open_in_editor", { path: worktreePath }).catch((error) => {
    console.error("open_in_editor failed:", error);
  });
}

/* ── Shared helpers ─────────────────────────────────────────────── */

function Field({
  label,
  value,
  status,
}: {
  label: string;
  value?: string;
  status?: string;
}) {
  const hasValue = Boolean(value?.trim());
  const displayValue = value?.trim() || EMPTY_VALUE;
  const statusLabel = status?.trim();

  return (
    <div className="code-work-panel__field">
      <dt>{label}</dt>
      <dd>
        <code className={!hasValue ? "code-work-panel__missing" : undefined} title={displayValue}>
          {displayValue}
        </code>
        {label === "Test report" ? (
          <span className={`code-work-panel__artifact-status code-work-panel__artifact-status--${artifactTone(statusLabel)}`}>
            {statusLabel ? statusLabel.toUpperCase() : "STATUS NOT REPORTED"}
          </span>
        ) : null}
      </dd>
    </div>
  );
}

function workerSummary(worker: WorkerRecord): string {
  return worker.summary ?? worker.lastText ?? worker.reason ?? "No worker summary published yet.";
}

function recentLogLines(chunks: string[]): string[] {
  return chunks
    .flatMap((chunk) => chunk.split(/\r?\n/))
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(-6);
}

function statusTone(status: WorkerRecord["status"]): "ok" | "warn" | "err" | "dim" {
  if (status === "completed") return "ok";
  if (status === "failed") return "err";
  if (status === "cancelled") return "dim";
  return "warn";
}

function artifactTone(status: string | undefined): "ok" | "warn" | "err" | "dim" {
  const normalized = status?.toLowerCase() ?? "";
  if (normalized.includes("pass") || normalized.includes("ok") || normalized.includes("success")) {
    return "ok";
  }
  if (normalized.includes("fail") || normalized.includes("error")) return "err";
  if (normalized.includes("run") || normalized.includes("pending")) return "warn";
  return "dim";
}

function compactId(id: string): string {
  return id.length > 28 ? `${id.slice(0, 25)}...` : id;
}
