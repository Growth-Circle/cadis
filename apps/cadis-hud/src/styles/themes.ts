/**
 * 6 hue presets for the HUD. Each theme drives a single CSS custom property
 * `--hue` on the document root; every other colour in `globals.css` is derived
 * from it via `oklch(L C var(--hue))`. This keeps theming to one var.
 *
 * Single source of truth — the `useHud` store re-exports `THEMES` for back-
 * compat, but new code should import from here.
 */
export type ThemeKey = "amber" | "arc" | "phosphor" | "violet" | "alert" | "ice";

export type ThemeDef = {
  key: ThemeKey;
  /** Display label shown in the picker. */
  label: string;
  /** OKLCH hue in degrees, plugged into `--hue`. */
  hue: number;
  /** Short description for accessibility / tooltips. */
  description: string;
};

export const THEMES: Record<ThemeKey, ThemeDef> = {
  arc: {
    key: "arc",
    label: "ARC REACTOR",
    hue: 210,
    description: "Cool blue — Iron-Man arc reactor",
  },
  amber: {
    key: "amber",
    label: "AMBER",
    hue: 38,
    description: "Warm amber CRT phosphor",
  },
  phosphor: {
    key: "phosphor",
    label: "PHOSPHOR",
    hue: 145,
    description: "Classic green terminal phosphor",
  },
  violet: {
    key: "violet",
    label: "VIOLET",
    hue: 290,
    description: "Cyberdeck violet",
  },
  alert: {
    key: "alert",
    label: "ALERT",
    hue: 18,
    description: "High-saturation red — incident mode",
  },
  ice: {
    key: "ice",
    label: "ICE",
    hue: 235,
    description: "Cold cyan — focus mode",
  },
};

export const THEME_ORDER: ThemeKey[] = ["arc", "amber", "phosphor", "violet", "alert", "ice"];

/** Cycle to the next theme in `THEME_ORDER`. Pure helper, easy to unit-test. */
export function nextTheme(current: ThemeKey): ThemeKey {
  const i = THEME_ORDER.indexOf(current);
  if (i === -1) return THEME_ORDER[0]!;
  return THEME_ORDER[(i + 1) % THEME_ORDER.length]!;
}

/** Apply a theme by writing `--hue` onto `document.documentElement`. */
export function applyTheme(key: ThemeKey, doc: Document = document): void {
  const t = THEMES[key];
  doc.documentElement.style.setProperty("--hue", String(t.hue));
}
