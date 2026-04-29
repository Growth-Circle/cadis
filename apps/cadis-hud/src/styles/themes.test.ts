import { describe, expect, it } from "vitest";
import { applyTheme, THEMES, THEME_ORDER, type ThemeKey } from "./themes.js";

describe("themes", () => {
  it("THEMES has all 6 expected keys", () => {
    const keys: ThemeKey[] = ["arc", "amber", "phosphor", "violet", "alert", "ice"];
    expect(Object.keys(THEMES).sort()).toEqual([...keys].sort());
  });

  it("THEME_ORDER contains exactly the 6 valid theme names", () => {
    expect(THEME_ORDER).toHaveLength(6);
    for (const key of THEME_ORDER) {
      expect(THEMES[key]).toBeDefined();
    }
  });

  it("applyTheme sets --hue CSS custom property on documentElement", () => {
    applyTheme("amber", document);
    expect(document.documentElement.style.getPropertyValue("--hue")).toBe("38");

    applyTheme("ice", document);
    expect(document.documentElement.style.getPropertyValue("--hue")).toBe("235");
  });
});
