import { useHud } from "../hudState.js";

const EQ_BAR_COUNT = 48;
const EQ_BARS = Array.from({ length: EQ_BAR_COUNT }, (_, i) => i);
const MAX_CENTER_NAME = 18;

export function RamaOrb() {
  const voice = useHud((s) => s.voiceState);
  const mainAgent = useHud((s) => s.agents.find((a) => a.spec.id === "main"));
  const mainName = mainAgent?.spec.name ?? "CADIS";
  const setRenameTarget = useHud((s) => s.setAgentRenameTarget);
  const displayName = compactAgentName(mainName).toUpperCase();
  const brandFontSize = Math.max(12, Math.min(26, Math.floor(176 / Math.max(4, displayName.length))));
  const brandLetterSpacing = displayName.length > 14 ? "0.04em" : displayName.length > 10 ? "0.10em" : "0.22em";
  const state = resolveOrbState(voice, mainAgent?.status ?? "idle", mainAgent?.currentTask.verb);

  return (
    <div
      className="rama-orb"
      data-state={state.animation}
      onContextMenu={(e) => {
        e.preventDefault();
        setRenameTarget("main");
      }}
    >
      <div className="rama-orb__rings">
        <div className="rama-orb__ring rama-orb__ring--outer" />
        <div className="rama-orb__ring rama-orb__ring--mid" />
        <div className="rama-orb__ring rama-orb__ring--inner" />
      </div>
      <div className="rama-orb__eq" aria-hidden="true">
        {EQ_BARS.map((i) => {
          const wave = 1 + ((i * 7) % 13);
          return (
            <span
              key={i}
              className="rama-orb__eq-bar"
              style={{
                ["--rot" as string]: `${(360 / EQ_BAR_COUNT) * i}deg`,
                ["--delay" as string]: `${-(i % 16) * 0.065}s`,
                ["--peak-idle" as string]: `${5 + (wave % 4)}px`,
                ["--peak-think" as string]: `${10 + wave}px`,
                ["--peak-work" as string]: `${12 + wave}px`,
                ["--peak-listen" as string]: `${15 + wave}px`,
                ["--peak-speak" as string]: `${20 + wave}px`,
              }}
            />
          );
        })}
      </div>
      <svg className="rama-orb__svg" viewBox="0 0 200 200">
        <circle className="rama-orb__arc" cx="100" cy="100" r="76" />
        <circle className="rama-orb__arc" cx="100" cy="100" r="62" style={{ animationDelay: "-2s", animationDuration: "8s" }} />
        <circle className="rama-orb__arc rama-orb__arc--wide" cx="100" cy="100" r="88" />
      </svg>
      <div className="rama-orb__core">
        <span
          className="rama-orb__brand"
          style={{ fontSize: `${brandFontSize}px`, letterSpacing: brandLetterSpacing }}
          title={mainName}
        >
          {displayName}
        </span>
        <span className="rama-orb__state">{state.label}</span>
      </div>
    </div>
  );
}

function resolveOrbState(
  voice: "idle" | "listening" | "thinking" | "speaking",
  status: "working" | "idle" | "waiting",
  verb?: string,
): { animation: string; label: string } {
  if (voice !== "idle") return { animation: voice, label: voice.toUpperCase() };
  if (status === "working") {
    const label = verb?.trim() ? verb.trim().toUpperCase() : "WORKING";
    return { animation: "working", label };
  }
  if (status === "waiting") return { animation: "waiting", label: "WAITING" };
  return { animation: "idle", label: "IDLE" };
}

function compactAgentName(name: string): string {
  const clean = name.trim().replace(/\s+/g, " ") || "CADIS";
  if (clean.length <= MAX_CENTER_NAME) return clean;
  return `${clean.slice(0, MAX_CENTER_NAME - 3)}...`;
}
