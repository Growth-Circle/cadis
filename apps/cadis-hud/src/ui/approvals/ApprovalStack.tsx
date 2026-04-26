/**
 * Stack of pending approval cards rendered in the HUD. Cards are added by the
 * gateway dispatcher (`approval.requested`) and removed when the server fans
 * out `approval.resolved`. Newest cards appear at the top.
 */
import { useHud, selectApprovals } from "../hudState.js";
import { ApprovalCard } from "./ApprovalCard.js";

export function ApprovalStack() {
  const approvals = useHud(selectApprovals);
  if (approvals.length === 0) return null;
  // Newest first.
  const ordered = [...approvals].sort((a, b) => b.ts - a.ts);
  return (
    <div className="approval-stack" role="region" aria-label="pending approvals">
      {ordered.map((a) => (
        <ApprovalCard key={a.id} approval={a} />
      ))}
    </div>
  );
}
