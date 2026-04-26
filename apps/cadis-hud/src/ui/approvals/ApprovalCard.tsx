/**
 * Inline approval card. Fired when the orchestrator publishes
 * `approval.requested`; the user clicks OK or DENY and we send
 * `{ type: "approval.respond", id, verdict }` back over the WS.
 *
 * The card is *not* removed locally on click — the server publishes
 * `approval.resolved` and the store reducer removes it from there. That keeps
 * Telegram/HUD/CLI surfaces in sync.
 */
import type { ApprovalRecord } from "../hudState.js";
import { sendApprovalResponse } from "../cadisActions.js";

export type ApprovalCardProps = {
  approval: ApprovalRecord;
  /** Optional override (used by tests) — defaults to the real gateway sender. */
  onRespond?: (id: string, verdict: "approve" | "deny") => boolean;
};

export function ApprovalCard({ approval, onRespond }: ApprovalCardProps) {
  const respond = onRespond ?? sendApprovalResponse;
  const cmdLabel = approval.cmd?.length > 120 ? `${approval.cmd.slice(0, 117)}…` : approval.cmd;
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
      <div className="approval-card__cmd" title={approval.cmd}>
        <span className="approval-card__verb">$</span>
        <code>{cmdLabel}</code>
      </div>
      {approval.cwd && <div className="approval-card__cwd">cwd · {approval.cwd}</div>}
      {approval.reason && <p className="approval-card__reason">{approval.reason}</p>}
      <footer className="approval-card__actions">
        <button
          type="button"
          className="approval-card__btn approval-card__btn--deny"
          onClick={() => respond(approval.id, "deny")}
        >
          DENY
        </button>
        <button
          type="button"
          className="approval-card__btn approval-card__btn--ok"
          onClick={() => respond(approval.id, "approve")}
        >
          OK
        </button>
      </footer>
    </article>
  );
}
