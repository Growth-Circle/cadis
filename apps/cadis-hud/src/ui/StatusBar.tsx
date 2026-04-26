import { useHud } from "./hudState.js";

const dotByGateway = {
  disconnected: "var(--err)",
  connecting: "var(--warn)",
  connected: "var(--ok)",
} as const;

export function StatusBar() {
  const gateway = useHud((s) => s.gateway);
  const agents = useHud((s) => s.agents);
  const defaultModel = useHud((s) => s.defaultModel);
  const agentModels = useHud((s) => s.agentModels);
  const mainName = agents.find((a) => a.spec.id === "main")?.spec.name ?? "CADIS";

  const counts = {
    working: agents.filter((a) => a.status === "working").length,
    waiting: agents.filter((a) => a.status === "waiting").length,
    idle: agents.filter((a) => a.status === "idle").length,
  };

  const cadisModel = agentModels.main ?? defaultModel ?? "-";

  return (
    <header className="status-bar">
      <span className="status-bar__brand">◈ {mainName.toUpperCase()} SYSTEM</span>
      <span className="status-bar__sep">│</span>
      <span className="status-bar__field">
        <span className="status-bar__dot" style={{ background: dotByGateway[gateway] }} />
        cadis · {gateway}
      </span>
      <span className="status-bar__sep">│</span>
      <span className="status-bar__field" style={{ color: "var(--accent)" }}>
        model · {cadisModel}
      </span>
      <span className="status-bar__sep">│</span>
      <span className="status-bar__field">
        <span className="status-bar__dot" style={{ background: "var(--ok)" }} />
        {counts.working} ACTIVE
      </span>
      <span className="status-bar__field">
        <span className="status-bar__dot" style={{ background: "var(--warn)" }} />
        {counts.waiting} WAITING
      </span>
      <span className="status-bar__field">
        <span className="status-bar__dot" style={{ background: "var(--faint)" }} />
        {counts.idle} IDLE
      </span>
    </header>
  );
}
