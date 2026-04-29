import { useHud, type AgentLive } from "../hudState.js";
import { WorkerTree } from "./WorkerTree.js";

const STATUS_DOT: Record<AgentLive["status"], string> = {
  working: "var(--ok)",
  idle: "var(--faint)",
  waiting: "var(--warn)",
};

export function AgentWidget({
  agent,
  xPct,
  yPct,
}: {
  agent: AgentLive;
  xPct: number;
  yPct: number;
}) {
  const accent = `oklch(0.75 0.16 ${agent.spec.hue})`;
  const setRenameTarget = useHud((s) => s.setAgentRenameTarget);
  return (
    <div
      className="agent-widget"
      onContextMenu={(e) => {
        e.preventDefault();
        setRenameTarget(agent.spec.id);
      }}
      style={{
        left: `${xPct}%`,
        top: `${yPct}%`,
        ["--agent-accent" as string]: accent,
      }}
    >
      <div className="agent-widget__head">
        <span className="agent-widget__icon">{agent.spec.icon}</span>
        <span className="agent-widget__name">{agent.spec.name}</span>
        <span className="agent-widget__status">
          <span
            className="agent-widget__dot"
            style={{ background: STATUS_DOT[agent.status] }}
          />
          {agent.status}
        </span>
      </div>
      <div className="agent-widget__role">{agent.spec.role} · {agent.specialist.label}</div>
      <div className="agent-widget__task">
        <span className="agent-widget__verb">{agent.currentTask.verb}</span>
        <span className="agent-widget__target">{agent.currentTask.target}</span>
      </div>
      <div className="agent-widget__detail">{agent.currentTask.detail}</div>
      <WorkerTree agentId={agent.spec.id} />
    </div>
  );
}
