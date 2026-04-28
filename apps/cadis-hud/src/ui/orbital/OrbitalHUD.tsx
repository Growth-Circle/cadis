/**
 * Orbital HUD layout — CADIS orb centered, agent widgets in 12 perimeter
 * slots. Layout uses a logical 1920×1080 coordinate space that scales to fit
 * the container while preserving aspect ratio.
 */
import { useMemo } from "react";
import { useHud } from "../hudState.js";
import { RamaOrb } from "./RamaOrb.js";
import { AgentWidget } from "./AgentWidget.js";

const W = 1920;
const H = 1080;

const SLOTS: { cx: number; cy: number }[] = [
  { cx: 380,  cy: 200 }, { cx: 700,  cy: 200 }, { cx: 1220, cy: 200 }, { cx: 1540, cy: 200 },
  { cx: 380,  cy: 420 },                                                  { cx: 1540, cy: 420 },
  { cx: 380,  cy: 660 },                                                  { cx: 1540, cy: 660 },
  { cx: 380,  cy: 880 }, { cx: 700,  cy: 880 }, { cx: 1220, cy: 880 }, { cx: 1540, cy: 880 },
];

export function OrbitalHUD() {
  const agents = useHud((s) => s.agents);
  const mainModel = useHud((s) => s.agentModels.main ?? s.defaultModel ?? "openai/gpt-5.5");
  const chatPrefs = useHud((s) => s.chatPreferences);
  const voiceState = useHud((s) => s.voiceState);
  const displayAgents = useMemo(
    () => agents.filter((agent) => agent.spec.id !== "main"),
    [agents],
  );
  const positions = useMemo(
    () =>
      displayAgents.map((_, i) => SLOTS[i] ?? SLOTS[SLOTS.length - 1]!),
    [displayAgents],
  );

  return (
    <div className="orbital-hud">
      <div className="orbital-hud__canvas">
        <svg
          className="orbital-hud__spokes"
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="xMidYMid meet"
        >
          <defs>
            <radialGradient id="ramaHalo">
              <stop offset="0%"   stopColor="oklch(0.78 0.16 var(--hue))" stopOpacity="0.18" />
              <stop offset="55%"  stopColor="oklch(0.78 0.16 var(--hue))" stopOpacity="0.06" />
              <stop offset="100%" stopColor="oklch(0.78 0.16 var(--hue))" stopOpacity="0" />
            </radialGradient>
          </defs>
          <circle cx={W / 2} cy={H / 2} r={620} fill="url(#ramaHalo)" />
          <ellipse
            cx={W / 2} cy={H / 2} rx={340} ry={220}
            fill="none"
            stroke="oklch(0.55 0.08 var(--hue) / 0.30)"
            strokeWidth={0.7}
            strokeDasharray="1 8"
          />
          <ellipse
            cx={W / 2} cy={H / 2} rx={520} ry={355}
            fill="none"
            stroke="oklch(0.55 0.08 var(--hue) / 0.22)"
            strokeWidth={0.7}
            strokeDasharray="1 10"
          />
          {displayAgents.map((a, i) => {
            const p = positions[i];
            if (!p) return null;
            return (
              <line
                key={a.spec.id}
                x1={W / 2} y1={H / 2} x2={p.cx} y2={p.cy}
                stroke={`oklch(0.6 0.13 ${a.spec.hue} / 0.45)`}
                strokeWidth={1}
                strokeDasharray="3 7"
              />
            );
          })}
        </svg>

        <div className="orbital-hud__core">
          <OrbMetaRing
            model={compactModelLabel(mainModel)}
            ctx={chatPrefs.fast ? "FAST · PRUNED" : "FULL · PRUNED"}
            mode={chatPrefs.thinking ? "THINK LOW" : "THINK OFF"}
            voice={voiceState}
          />
          <RamaOrb />
        </div>

        {displayAgents.map((a, i) => {
          const p = positions[i];
          if (!p) return null;
          return (
            <AgentWidget
              key={a.spec.id}
              agent={a}
              xPct={(p.cx / W) * 100}
              yPct={(p.cy / H) * 100}
            />
          );
        })}
      </div>
    </div>
  );
}

function OrbMetaRing({
  model,
  ctx,
  mode,
  voice,
}: {
  model: string;
  ctx: string;
  mode: string;
  voice: string;
}) {
  return (
    <div className="orbital-hud__meta-ring" aria-hidden="true">
      <MetaItem className="orbital-hud__meta-item--model" label="MODEL" value={model} />
      <MetaItem className="orbital-hud__meta-item--ctx" label="CTX" value={ctx} />
      <MetaItem className="orbital-hud__meta-item--mode" label="MODE" value={mode} />
      <MetaItem className="orbital-hud__meta-item--voice" label="VOICE" value={voice.toUpperCase()} />
    </div>
  );
}

function MetaItem({
  className,
  label,
  value,
}: {
  className: string;
  label: string;
  value: string;
}) {
  return (
    <div className={`orbital-hud__meta-item ${className}`}>
      <span>{label}</span>
      <strong title={value}>{value}</strong>
    </div>
  );
}

function compactModelLabel(model: string): string {
  const clean = model.replace(/^openai-codex\//, "").replace(/^openai\//, "");
  return clean.length > 18 ? `${clean.slice(0, 15)}...` : clean;
}
