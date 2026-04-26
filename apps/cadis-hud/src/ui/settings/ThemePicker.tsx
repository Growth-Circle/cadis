/**
 * Compact theme picker — clicking the swatch cycles through the 6 hue presets.
 * Long-press / right-click would open a full grid; for now a single button is
 * enough to satisfy P5.6 ("cycling themes works at runtime").
 */
import { useHud } from "../hudState.js";
import { THEMES, THEME_ORDER, nextTheme, type ThemeKey } from "../../styles/themes.js";
import { persistThemePreference } from "../cadisActions.js";

export function ThemePicker() {
  const theme = useHud((s) => s.theme);
  const setTheme = useHud((s) => s.setTheme);
  const def = THEMES[theme];
  const pickTheme = (next: ThemeKey) => {
    setTheme(next);
    persistThemePreference(next);
  };

  return (
    <div className="theme-picker" role="group" aria-label="theme">
      <button
        type="button"
        className="theme-picker__btn"
        title={`${def.label} — click to cycle`}
        aria-label={`theme: ${def.label}, click to cycle`}
        onClick={() => pickTheme(nextTheme(theme))}
      >
        <span
          className="theme-picker__swatch"
          style={{ background: `oklch(0.78 0.16 ${def.hue})` }}
        />
        <span className="theme-picker__label">{def.label}</span>
      </button>
      <ul className="theme-picker__grid" role="listbox" aria-label="theme presets">
        {THEME_ORDER.map((k) => (
          <ThemeDot key={k} k={k} active={k === theme} onPick={pickTheme} />
        ))}
      </ul>
    </div>
  );
}

function ThemeDot({
  k,
  active,
  onPick,
}: {
  k: ThemeKey;
  active: boolean;
  onPick: (k: ThemeKey) => void;
}) {
  const def = THEMES[k];
  return (
    <li className="theme-picker__dot-wrap">
      <button
        type="button"
        role="option"
        aria-selected={active}
        title={def.label}
        className={`theme-picker__dot${active ? " theme-picker__dot--active" : ""}`}
        style={{ background: `oklch(0.78 0.16 ${def.hue})` }}
        onClick={() => onPick(k)}
      />
    </li>
  );
}
