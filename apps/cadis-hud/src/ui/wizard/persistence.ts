/**
 * Persistence helper for the first-run wizard.
 *
 * Settings are written to `~/.config/cadis/settings.json` via the Tauri
 * `@tauri-apps/plugin-fs` API. Outside of Tauri (e.g. unit tests, plain
 * `pnpm dev`), the function is a no-op so the wizard can still run in the
 * browser harness for visual development.
 *
 * Keep this module side-effect free at import time — the Tauri plugin is
 * `await import()`ed lazily so vitest/jsdom can mock the whole module.
 */

export type WizardSettings = {
  /** Theme key from `src/ui/hudState.ts` THEMES (e.g. "arc", "amber"). */
  theme: string;
  /** Whether the user opted in to wake-word + STT/TTS. */
  voiceEnabled: boolean;
  /** Whether to keep the Telegram approval bridge active alongside the HUD. */
  telegramFallback: boolean;
  /** Approval timeout in seconds. Default 300 (= 5 min). */
  approvalTimeoutSec: number;
  /** Display-only — actual binding is configured in the Tauri shell. */
  hotkey: string;
};

export const DEFAULT_WIZARD_SETTINGS: WizardSettings = {
  theme: "arc",
  voiceEnabled: false,
  telegramFallback: true,
  approvalTimeoutSec: 300,
  hotkey: "Super+Space",
};

const RELATIVE_PATH = ".config/cadis/settings.json";

/**
 * Persist wizard choices. Resolves once the file is written or skipped.
 *
 * Returns:
 *   - "saved"   — write succeeded
 *   - "skipped" — running outside Tauri (no fs plugin available)
 *   - "error"   — write failed; details forwarded via the optional logger
 */
export type SaveOutcome = "saved" | "skipped" | "error";

export async function saveWizardSettings(
  settings: WizardSettings,
  logger?: { warn: (msg: string, meta?: Record<string, unknown>) => void },
): Promise<SaveOutcome> {
  let fs: typeof import("@tauri-apps/plugin-fs");
  try {
    fs = await import("@tauri-apps/plugin-fs");
  } catch {
    return "skipped";
  }
  try {
    // Ensure parent dir exists; ignore "already exists" errors.
    try {
      await fs.mkdir(".config/cadis", {
        baseDir: fs.BaseDirectory.Home,
        recursive: true,
      });
    } catch {
      /* directory already exists */
    }
    await fs.writeTextFile(RELATIVE_PATH, JSON.stringify(settings, null, 2), {
      baseDir: fs.BaseDirectory.Home,
    });
    return "saved";
  } catch (err) {
    logger?.warn("[wizard] saveWizardSettings failed", {
      error: err instanceof Error ? err.message : String(err),
    });
    return "error";
  }
}
