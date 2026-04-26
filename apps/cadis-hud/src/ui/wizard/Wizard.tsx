/**
 * First-run wizard. Five short steps that capture the choices the user must
 * make before the HUD becomes their primary surface (spec §14, checklist
 * "Decision sign-off"). Selections persist to `~/.config/cadis/settings.json`
 * via `saveWizardSettings`.
 *
 * Caller decides when to mount this. A typical entry point:
 *   const settings = await readWizardSettings();
 *   if (!settings) <Wizard onDone={...} />
 *
 * Persistence is decoupled (see `persistence.ts`) so this component is easy
 * to test without Tauri.
 */
import { useState } from "react";
import {
  DEFAULT_WIZARD_SETTINGS,
  saveWizardSettings,
  type WizardSettings,
  type SaveOutcome,
} from "./persistence.js";

type SaveFn = (s: WizardSettings) => Promise<SaveOutcome>;

export type WizardProps = {
  /** Called once Save resolves. Receives the saved settings + outcome. */
  onDone?: (s: WizardSettings, outcome: SaveOutcome) => void;
  /** Optional override for the persistence function — used by tests. */
  save?: SaveFn;
  /** Optional initial values — defaults to `DEFAULT_WIZARD_SETTINGS`. */
  initial?: Partial<WizardSettings>;
};

const THEME_CHOICES: { id: string; label: string }[] = [
  { id: "arc", label: "Arc Reactor (default)" },
  { id: "amber", label: "Amber" },
  { id: "phosphor", label: "Phosphor" },
  { id: "violet", label: "Violet" },
  { id: "alert", label: "Alert" },
  { id: "ice", label: "Ice" },
];

const STEP_TITLES = [
  "Pick a theme",
  "Voice mode",
  "Telegram fallback",
  "Approval timeout",
  "Hotkey",
];

export function Wizard({ onDone, save, initial }: WizardProps) {
  const [step, setStep] = useState(0);
  const [settings, setSettings] = useState<WizardSettings>({
    ...DEFAULT_WIZARD_SETTINGS,
    ...(initial ?? {}),
  });
  const [saving, setSaving] = useState(false);
  const [outcome, setOutcome] = useState<SaveOutcome | null>(null);

  const update = <K extends keyof WizardSettings>(k: K, v: WizardSettings[K]) =>
    setSettings((s) => ({ ...s, [k]: v }));

  const handleSave = async () => {
    setSaving(true);
    const fn = save ?? saveWizardSettings;
    const result = await fn(settings);
    setOutcome(result);
    setSaving(false);
    onDone?.(settings, result);
  };

  return (
    <div className="rama-wizard" role="dialog" aria-label="CADIS first-run wizard">
      <header className="rama-wizard__header">
        <h2>CADIS setup</h2>
        <p>
          Step {step + 1} of {STEP_TITLES.length}: {STEP_TITLES[step]}
        </p>
      </header>

      <section className="rama-wizard__body">
        {step === 0 && (
          <fieldset>
            <legend>Theme</legend>
            {THEME_CHOICES.map((t) => (
              <label key={t.id} className="rama-wizard__row">
                <input
                  type="radio"
                  name="theme"
                  value={t.id}
                  checked={settings.theme === t.id}
                  onChange={() => update("theme", t.id)}
                />
                {t.label}
              </label>
            ))}
          </fieldset>
        )}

        {step === 1 && (
          <fieldset>
            <legend>Voice mode</legend>
            <label className="rama-wizard__row">
              <input
                type="checkbox"
                checked={settings.voiceEnabled}
                onChange={(e) => update("voiceEnabled", e.target.checked)}
              />
              Enable wake word + spoken replies (in-progress, requires helper services)
            </label>
            <p className="rama-wizard__hint">
              You can flip this later from the voice config panel. Helper services
              are installed manually — see the HUD README under &quot;Voice mode&quot;.
            </p>
          </fieldset>
        )}

        {step === 2 && (
          <fieldset>
            <legend>Telegram fallback</legend>
            <label className="rama-wizard__row">
              <input
                type="checkbox"
                checked={settings.telegramFallback}
                onChange={(e) => update("telegramFallback", e.target.checked)}
              />
              Keep Telegram approvals active alongside the HUD
            </label>
            <p className="rama-wizard__hint">
              Recommended on. The HUD wins approvals when present, but Telegram
              still receives them so you can approve from your phone.
            </p>
          </fieldset>
        )}

        {step === 3 && (
          <fieldset>
            <legend>Approval timeout</legend>
            <label className="rama-wizard__row">
              Seconds before an unanswered approval auto-denies:
              <input
                type="number"
                min={30}
                max={3600}
                value={settings.approvalTimeoutSec}
                onChange={(e) =>
                  update(
                    "approvalTimeoutSec",
                    Math.max(30, Math.min(3600, Number(e.target.value) || 300)),
                  )
                }
              />
            </label>
            <p className="rama-wizard__hint">Default 300 s (5 minutes).</p>
          </fieldset>
        )}

        {step === 4 && (
          <fieldset>
            <legend>Global hotkey</legend>
            <p>
              The HUD answers to <code>{settings.hotkey}</code> system-wide. To
              change the binding, edit <code>src-tauri/src/lib.rs</code> and
              rebuild — the Tauri global-shortcut plugin is the source of
              truth.
            </p>
          </fieldset>
        )}
      </section>

      <footer className="rama-wizard__footer">
        <button
          type="button"
          disabled={step === 0 || saving}
          onClick={() => setStep((s) => Math.max(0, s - 1))}
        >
          Back
        </button>
        {step < STEP_TITLES.length - 1 && (
          <button
            type="button"
            disabled={saving}
            onClick={() => setStep((s) => Math.min(STEP_TITLES.length - 1, s + 1))}
          >
            Next
          </button>
        )}
        {step === STEP_TITLES.length - 1 && (
          <button type="button" disabled={saving} onClick={handleSave}>
            {saving ? "Saving…" : "Save"}
          </button>
        )}
        {outcome && (
          <span className="rama-wizard__status" role="status">
            {outcome === "saved" && "Saved."}
            {outcome === "skipped" && "Skipped (running outside Tauri)."}
            {outcome === "error" && "Save failed — see console."}
          </span>
        )}
      </footer>
    </div>
  );
}
