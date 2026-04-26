import { describe, expect, it } from "vitest";
import { normalizeAgentName, useHud } from "./hudState.js";

describe("hudState", () => {
  it("normalizes CADIS display names", () => {
    expect(normalizeAgentName("  CADIS   Prime  ")).toBe("CADIS Prime");
    expect(normalizeAgentName("", "Research")).toBe("Research");
  });

  it("seeds the main CADIS agent", () => {
    const main = useHud.getState().agents.find((agent) => agent.spec.id === "main");
    expect(main?.spec.name).toBe("CADIS");
  });
});
