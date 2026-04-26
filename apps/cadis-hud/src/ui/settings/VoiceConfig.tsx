/**
 * Voice configuration modal — pick voice, tune rate/pitch/volume, test playback.
 * Persists through the CADIS daemon preference protocol when connected.
 */
import { useState } from "react";
import { useHud } from "../hudState.js";
import { VOICES } from "../../lib/voice/voices.js";
import { stopSpeaking, testAudio } from "../../lib/voice/tts.js";
import { persistVoicePreferences } from "../cadisActions.js";

export function VoiceConfig() {
  const open = useHud((s) => s.voiceConfigOpen);
  const close = useHud((s) => s.setVoiceConfigOpen);
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

  if (!open) return null;

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
    <div className="modal-backdrop" onClick={() => close(false)}>
      <div className="voice-config" onClick={(e) => e.stopPropagation()}>
        <header className="voice-config__head">
          <span className="voice-config__brand">VOICE · CONFIG</span>
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
          hint="Speed of speech"
        />
        <SliderRow
          label="Pitch"
          unit="Hz"
          min={-50}
          max={50}
          step={5}
          value={prefs.pitch}
          onChange={(v) => updateVoice({ pitch: v })}
          hint="Higher / lower tone"
        />
        <SliderRow
          label="Volume"
          unit="%"
          min={-50}
          max={50}
          step={5}
          value={prefs.volume}
          onChange={(v) => updateVoice({ volume: v })}
          hint="Output gain"
        />

        <section className="voice-config__row">
          <label className="voice-config__label">
            <input
              type="checkbox"
              checked={prefs.autoSpeak}
              onChange={(e) => updateVoice({ autoSpeak: e.target.checked })}
            />
            Auto-speak CADIS chat replies
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

        <footer className="voice-config__foot">
          <button
            type="button"
            className="voice-config__btn"
            onClick={testing ? stop : test}
          >
            {testing ? "STOP" : "TEST"}
          </button>
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

function SliderRow(props: {
  label: string;
  unit: string;
  min: number;
  max: number;
  step: number;
  value: number;
  onChange: (v: number) => void;
  hint?: string;
}) {
  const { label, unit, min, max, step, value, onChange, hint } = props;
  const sign = value >= 0 ? "+" : "";
  return (
    <section className="voice-config__row">
      <label className="voice-config__label">
        {label}
        <span className="voice-config__value">{sign}{value}{unit}</span>
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
      {hint && <div className="voice-config__hint">{hint}</div>}
    </section>
  );
}
