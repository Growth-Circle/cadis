/**
 * Models config — pick which model each registered agent uses.
 * Sources the catalog from CADIS `models.list.response` frames.
 */
import { useHud, THEMES, type ThemeKey } from "../hudState.js";
import {
  persistBackgroundOpacityPreference,
  persistThemePreference,
  sendAgentModelUpdate,
} from "../cadisActions.js";

export function ModelsConfig() {
  const open = useHud((s) => s.modelsConfigOpen);
  const close = useHud((s) => s.setModelsConfigOpen);
  const models = useHud((s) => s.availableModels);
  const defaultModel = useHud((s) => s.defaultModel);
  const agents = useHud((s) => s.agents);
  const agentModels = useHud((s) => s.agentModels);
  const theme = useHud((s) => s.theme);
  const setTheme = useHud((s) => s.setTheme);
  const backgroundOpacity = useHud((s) => s.backgroundOpacity);
  const setBackgroundOpacity = useHud((s) => s.setBackgroundOpacity);
  const pickTheme = (next: ThemeKey) => {
    setTheme(next);
    persistThemePreference(next);
  };
  const updateBackgroundOpacity = (next: number) => {
    setBackgroundOpacity(next);
    persistBackgroundOpacityPreference(next);
  };

  if (!open) return null;

  return (
    <div className="modal-backdrop" onClick={() => close(false)}>
      <div className="voice-config" onClick={(e) => e.stopPropagation()} style={{ width: 540 }}>
        <header className="voice-config__head">
          <span className="voice-config__brand">CONFIG · MODELS · THEME</span>
          <button
            type="button"
            className="voice-config__close"
            onClick={() => close(false)}
            aria-label="close"
          >
            ×
          </button>
        </header>

        <section className="voice-config__row">
          <label className="voice-config__label">
            Theme
            <span className="voice-config__value">{THEMES[theme].label}</span>
          </label>
          <div className="status-bar__themes" style={{ flexWrap: "wrap", gap: 4 }}>
            {(Object.keys(THEMES) as ThemeKey[]).map((k) => (
              <button
                key={k}
                type="button"
                className={`status-bar__theme${k === theme ? " status-bar__theme--active" : ""}`}
                style={{ ["--theme-hue" as string]: String(THEMES[k].hue) }}
                onClick={() => pickTheme(k)}
              >
                {THEMES[k].label}
              </button>
            ))}
          </div>
        </section>

        <section className="voice-config__row">
          <label className="voice-config__label">
            Background opacity
            <span className="voice-config__value">{backgroundOpacity}%</span>
          </label>
          <input
            type="range"
            className="voice-config__slider"
            min={15}
            max={100}
            step={5}
            value={backgroundOpacity}
            onChange={(e) => updateBackgroundOpacity(Number(e.target.value))}
          />
        </section>

        <section className="voice-config__row">
          <label className="voice-config__label">
            Available models
            <span className="voice-config__value">{models.length}</span>
          </label>
          <div className="voice-config__hint">
            Default from CADIS: <code>{defaultModel ?? "—"}</code>
          </div>
        </section>

        <section className="voice-config__row">
          <label className="voice-config__label">Per-agent model</label>
          <div className="agent-models">
            {agents.map((a) => {
              const current = agentModels[a.spec.id] ?? defaultModel ?? "";
              return (
                <div key={a.spec.id} className="agent-models__row">
                  <span className="agent-models__name">{a.spec.name}</span>
                  <select
                    className="voice-config__select"
                    value={current}
                    onChange={(e) => {
                      const nextModel = e.target.value;
                      sendAgentModelUpdate(a.spec.id, nextModel);
                    }}
                  >
                    {!models.includes(current) && current && (
                      <option value={current}>{current} (current)</option>
                    )}
                    {models.length === 0 && (
                      <option value="">— waiting for catalog —</option>
                    )}
                    {models.map((m) => (
                      <option key={m} value={m}>{m}</option>
                    ))}
                  </select>
                </div>
              );
            })}
          </div>
          <div className="voice-config__hint">
            Saved per-agent. Empty = use CADIS default.
          </div>
        </section>

        <footer className="voice-config__foot">
          <button
            type="button"
            className="voice-config__btn voice-config__btn--primary"
            onClick={() => close(false)}
          >
            DONE
          </button>
        </footer>
      </div>
    </div>
  );
}
