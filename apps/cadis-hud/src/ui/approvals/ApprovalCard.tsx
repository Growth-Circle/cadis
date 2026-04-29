/**
 * Inline approval card. Fired when the orchestrator publishes
 * `approval.requested`; the user clicks OK or DENY and we send
 * `{ type: "approval.respond", id, verdict }` back over the WS.
 *
 * The card is *not* removed locally on click — the server publishes
 * `approval.resolved` and the store reducer removes it from there. That keeps
 * Telegram/HUD/CLI surfaces in sync.
 */
import { useState, useEffect } from "react";
import { useHud, type ApprovalRecord } from "../hudState.js";
import { sendApprovalResponse } from "../cadisActions.js";

export type ApprovalCardProps = {
  approval: ApprovalRecord;
  /** Optional override (used by tests) — defaults to the real gateway sender. */
  onRespond?: (id: string, verdict: "approve" | "deny") => boolean;
};

function formatRemaining(ms: number): string {
  if (ms <= 0) return "Expired";
  const totalSec = Math.ceil(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  return `${min}:${String(sec).padStart(2, "0")} remaining`;
}

function useExpiry(expiresAt: string | undefined): { label: string; expired: boolean } {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!expiresAt) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [expiresAt]);
  if (!expiresAt) return { label: "", expired: false };
  const ms = new Date(expiresAt).getTime() - now;
  return { label: formatRemaining(ms), expired: ms <= 0 };
}

export function ApprovalCard({ approval, onRespond }: ApprovalCardProps) {
  const gateway = useHud((s) => s.gateway);
  const respond = onRespond ?? sendApprovalResponse;
  const cmdLabel = approval.cmd?.length > 120 ? `${approval.cmd.slice(0, 117)}…` : approval.cmd;
  const { label: expiryLabel, expired } = useExpiry(approval.expiresAt);
  const disconnected = gateway !== "connected";
  return (
    <article
      className="approval-card"
      data-approval-id={approval.id}
      aria-label={`approval ${approval.id}`}
    >
      <header className="approval-card__head">
        <span className="approval-card__rule">{approval.ruleId || "approval"}</span>
        <span className="approval-card__agent">{approval.agentId}</span>
      </header>
      {approval.summary && (
        <p className="approval-card__summary">{approval.summary}</p>
      )}
      <div className="approval-card__cmd" title={approval.cmd}>
        <span className="approval-card__verb">$</span>
        <code>{cmdLabel}</code>
      </div>
      {approval.cwd && <div className="approval-card__cwd">cwd · {approval.cwd}</div>}
      {approval.reason && <p className="approval-card__reason">{approval.reason}</p>}
      {expiryLabel && (
        <div className={`approval-card__expiry${expired ? " approval-card__expiry--expired" : ""}`}>
          {expiryLabel}
        </div>
      )}
      <footer className="approval-card__actions" style={disconnected ? { opacity: 0.45 } : undefined}>
        {disconnected && (
          <span className="approval-card__disconnect-hint">daemon disconnected</span>
        )}
        <button
          type="button"
          className="approval-card__btn approval-card__btn--deny"
          disabled={expired || disconnected}
          onClick={() => respond(approval.id, "deny")}
          title={disconnected ? "Daemon disconnected" : undefined}
        >
          DENY
        </button>
        <button
          type="button"
          className="approval-card__btn approval-card__btn--ok"
          disabled={expired || disconnected}
          onClick={() => respond(approval.id, "approve")}
          title={disconnected ? "Daemon disconnected" : undefined}
        >
          OK
        </button>
      </footer>
    </article>
  );
}
