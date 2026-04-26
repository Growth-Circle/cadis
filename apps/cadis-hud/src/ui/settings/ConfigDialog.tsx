import { useState } from "react";
import {
  THEMES,
  useHud,
  type AvatarStyle,
  type ConfigTab,
  type ThemeKey,
} from "../hudState.js";
import { VOICES } from "../../lib/voice/voices.js";
import { stopSpeaking, testAudio } from "../../lib/voice/tts.js";
import {
  persistBackgroundOpacityPreference,
  persistAvatarStylePreference,
  persistChatPreferences,
  persistThemePreference,
  persistVoicePreferences,
  sendAgentModelUpdate,
} from "../cadisActions.js";

const TABS: { id: ConfigTab; label: string }[] = [
  { id: "voice", label: "Voice" },
  { id: "models", label: "Models" },
  { id: "appearance", label: "Appearance" },
  { id: "window", label: "Window" },
];

const AVATAR_STYLES: { id: AvatarStyle; label: string; detail: string }[] = [
  { id: "orb", label: "CADIS Orb", detail: "Current RamaClaw-style core" },
  { id: "wulan_arc", label: "Wulan Arc", detail: "Hologram avatar contribution" },
];

export function ConfigDialog() {
  const open = useHud((s) => s.configOpen);
  const tab = useHud((s) => s.configTab);
  const setOpen = useHud((s) => s.setConfigOpen);
  const setTab = useHud((s) => s.setConfigTab);

  if (!open) return null;

  return (
    <div className="modal-backdrop" onClick={() => setOpen(false)}>
      <div className="voice-config config-dialog" onClick={(e) => e.stopPropagation()}>
        <header className="voice-config__head">
          <span className="voice-config__brand">WINDOW · CONFIGURE</span>
          <button
            type="button"
            className="voice-config__close"
            onClick={() => setOpen(false)}
            aria-label="close"
          >
            ×
          </button>
        </header>

        <nav className="config-tabs" aria-label="configuration sections">
          {TABS.map((item) => (
            <button
              key={item.id}
              type="button"
              className={`config-tabs__btn${tab === item.id ? " config-tabs__btn--active" : ""}`}
              onClick={() => setTab(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>

        <div className="config-dialog__body">
          {tab === "voice" && <VoiceTab />}
          {tab === "models" && <ModelsTab />}
          {tab === "appearance" && <AppearanceTab />}
          {tab === "window" && <WindowTab />}
        </div>

        <footer className="voice-config__foot">
          <button
            type="button"
            className="voice-config__btn voice-config__btn--primary"
            onClick={() => setOpen(false)}
          >
            DONE
          </button>
        </footer>
      </div>
    </div>
  );
}

function VoiceTab() {
  const prefs = useHud((s) => s.voicePrefs);
  const update = useHud((s) => s.updateVoicePrefs);
  const setVoiceState = useHud((s) => s.setVoiceState);
  const mainName = useHud((s) => s.agents.find((a) => a.spec.id === "main")?.spec.name ?? "CADIS");
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastEngine, setLastEngine] = useState<string | null>(null);
  const updateVoice = (patch: Partial<typeof prefs>) => {
    const next = { ...prefs, ...patch };
    update(patch);
    persistVoicePreferences(next);
  };

  const test = async () => {
    setError(null);
    setLastEngine(null);
    setTesting(true);
    setVoiceState("speaking");
    try {
      const engine = await testAudio(prefs, {
        onEnd: () => {
          setTesting(false);
          setVoiceState("idle");
        },
      }, mainName);
      setLastEngine(engine);
    } catch (e) {
      setTesting(false);
      setVoiceState("idle");
      setError(e instanceof Error ? e.message : "test failed");
    }
  };

  const stop = async () => {
    await stopSpeaking();
    setTesting(false);
    setVoiceState("idle");
  };

  return (
    <>
      <section className="voice-config__row">
        <label className="voice-config__label">Voice</label>
        <select
          className="voice-config__select"
          value={prefs.voiceId}
          onChange={(e) => updateVoice({ voiceId: e.target.value })}
        >
          {VOICES.map((v) => (
            <option key={v.id} value={v.id}>
              {v.label}
            </option>
          ))}
        </select>
      </section>

      <SliderRow
        label="Rate"
        unit="%"
        min={-50}
        max={50}
        step={5}
        value={prefs.rate}
        onChange={(v) => updateVoice({ rate: v })}
      />
      <SliderRow
        label="Pitch"
        unit="Hz"
        min={-50}
        max={50}
        step={5}
        value={prefs.pitch}
        onChange={(v) => updateVoice({ pitch: v })}
      />
      <SliderRow
        label="Volume"
        unit="%"
        min={-50}
        max={50}
        step={5}
        value={prefs.volume}
        onChange={(v) => updateVoice({ volume: v })}
      />

      <section className="voice-config__row">
        <label className="voice-config__label">
          <input
            type="checkbox"
            checked={prefs.autoSpeak}
            onChange={(e) => updateVoice({ autoSpeak: e.target.checked })}
          />
          Auto-speak chat replies
        </label>
      </section>

      <section className="voice-config__row">
        <label className="voice-config__label">
          Engine
          <span className="voice-config__value">edge-tts-universal</span>
        </label>
      </section>

      {error && <div className="voice-config__error">{error}</div>}
      {lastEngine && (
        <div className="voice-config__hint" style={{ color: "var(--ok)" }}>
          played via <code>{lastEngine}</code>
        </div>
      )}

      <div className="config-dialog__actions">
        <button
          type="button"
          className="voice-config__btn"
          onClick={testing ? stop : test}
        >
          {testing ? "STOP" : "TEST"}
        </button>
      </div>
    </>
  );
}

function ModelsTab() {
  const models = useHud((s) => s.availableModels);
  const defaultModel = useHud((s) => s.defaultModel);
  const agents = useHud((s) => s.agents);
  const agentModels = useHud((s) => s.agentModels);
  const chatPrefs = useHud((s) => s.chatPreferences);
  const setChatPreferences = useHud((s) => s.setChatPreferences);
  const updateChatPrefs = (patch: Partial<typeof chatPrefs>) => {
    const next = { ...chatPrefs, ...patch };
    setChatPreferences(patch);
    persistChatPreferences(next);
  };

  return (
    <>
      <section className="voice-config__row">
        <label className="voice-config__label">
          Available models
          <span className="voice-config__value">{models.length}</span>
        </label>
        <div className="voice-config__hint">
          Default from CADIS: <code>{defaultModel ?? "-"}</code>
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
                  {models.length === 0 && <option value="">waiting for catalog</option>}
                  {models.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              </div>
            );
          })}
        </div>
      </section>
      <section className="voice-config__row">
        <label className="voice-config__label">Thinking mode</label>
        <label>
          <input
            type="checkbox"
            checked={chatPrefs.thinking}
            onChange={(e) => updateChatPrefs({ thinking: e.target.checked })}
          />
          Enable thinking mode
        </label>
      </section>
      <section className="voice-config__row">
        <label className="voice-config__label">Speed mode</label>
        <label>
          <input
            type="checkbox"
            checked={chatPrefs.fast}
            onChange={(e) => updateChatPrefs({ fast: e.target.checked })}
          />
          Fast responses
        </label>
      </section>
    </>
  );
}

function AppearanceTab() {
  const theme = useHud((s) => s.theme);
  const avatarStyle = useHud((s) => s.avatarStyle);
  const setTheme = useHud((s) => s.setTheme);
  const setAvatarStyle = useHud((s) => s.setAvatarStyle);
  const pickTheme = (next: ThemeKey) => {
    setTheme(next);
    persistThemePreference(next);
  };
  const pickAvatar = (next: AvatarStyle) => {
    setAvatarStyle(next);
    persistAvatarStylePreference(next);
  };

  return (
    <>
      <section className="voice-config__row">
        <label className="voice-config__label">
          Avatar
          <span className="voice-config__value">
            {AVATAR_STYLES.find((style) => style.id === avatarStyle)?.label}
          </span>
        </label>
        <div className="config-avatar-grid">
          {AVATAR_STYLES.map((style) => (
            <button
              key={style.id}
              type="button"
              className={`config-avatar${style.id === avatarStyle ? " config-avatar--active" : ""}`}
              onClick={() => pickAvatar(style.id)}
            >
              <span className={`config-avatar__preview config-avatar__preview--${style.id}`} />
              <span className="config-avatar__copy">
                <strong>{style.label}</strong>
                <small>{style.detail}</small>
              </span>
            </button>
          ))}
        </div>
      </section>

      <section className="voice-config__row">
        <label className="voice-config__label">
          Theme
          <span className="voice-config__value">{THEMES[theme].label}</span>
        </label>
        <div className="config-theme-grid">
          {(Object.keys(THEMES) as ThemeKey[]).map((k) => (
            <button
              key={k}
              type="button"
              className={`config-theme${k === theme ? " config-theme--active" : ""}`}
              style={{ ["--theme-hue" as string]: String(THEMES[k].hue) }}
              onClick={() => pickTheme(k)}
            >
              <span className="config-theme__swatch" />
              {THEMES[k].label}
            </button>
          ))}
        </div>
      </section>
    </>
  );
}

function WindowTab() {
  const backgroundOpacity = useHud((s) => s.backgroundOpacity);
  const setBackgroundOpacity = useHud((s) => s.setBackgroundOpacity);
  const updateBackgroundOpacity = (next: number) => {
    setBackgroundOpacity(next);
    persistBackgroundOpacityPreference(next);
  };

  return (
    <>
      <section className="voice-config__row">
        <label className="voice-config__label">
          Transparent background
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
          Window frame
          <span className="voice-config__value">transparent</span>
        </label>
      </section>
    </>
  );
}

function SliderRow(props: {
  label: string;
  unit: string;
  min: number;
  max: number;
  step: number;
  value: number;
  onChange: (v: number) => void;
}) {
  const { label, unit, min, max, step, value, onChange } = props;
  const sign = value >= 0 ? "+" : "";
  return (
    <section className="voice-config__row">
      <label className="voice-config__label">
        {label}
        <span className="voice-config__value">
          {sign}
          {value}
          {unit}
        </span>
      </label>
      <input
        type="range"
        className="voice-config__slider"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </section>
  );
}
