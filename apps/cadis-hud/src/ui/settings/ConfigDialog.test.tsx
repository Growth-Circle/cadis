import "@testing-library/jest-dom/vitest";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { createElement } from "react";
import { useHud } from "../hudState.js";
import { ConfigDialog } from "./ConfigDialog.js";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../lib/voice/tts.js", () => ({
  testAudio: vi.fn(),
  stopSpeaking: vi.fn(),
}));
vi.mock("../cadisActions.js", () => ({
  persistBackgroundOpacityPreference: vi.fn(),
  persistAvatarStylePreference: vi.fn(),
  persistChatPreferences: vi.fn(),
  persistThemePreference: vi.fn(),
  persistVoicePreferences: vi.fn(),
  requestVoiceDoctor: vi.fn(),
  sendVoicePreflight: vi.fn(),
  sendAgentModelUpdate: vi.fn(),
}));

beforeEach(() => {
  useHud.setState({ configOpen: true, configTab: "voice" });
});

describe("ConfigDialog", () => {
  it("renders all four tabs", () => {
    render(createElement(ConfigDialog));
    const nav = screen.getByRole("navigation", { name: /configuration sections/i });
    expect(nav).toBeInTheDocument();
    const buttons = nav.querySelectorAll("button");
    const labels = Array.from(buttons).map((b) => b.textContent);
    expect(labels).toEqual(["Voice", "Models", "Appearance", "Window"]);
  });
});
