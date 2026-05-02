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

  it("disables model mutation controls while disconnected", () => {
    useHud.setState({
      configOpen: true,
      configTab: "models",
      gateway: "disconnected",
      defaultModel: "openai/gpt-5.5",
      availableModels: ["openai/gpt-5.5"],
      agentModels: { main: "openai/gpt-5.5" },
      chatPreferences: { thinking: false, fast: false },
      agents: [
        {
          spec: { id: "main", name: "CADIS", role: "Core", icon: "C", hue: 180, tasks: [] },
          status: "idle",
          currentTask: { verb: "ready", target: "CADIS", detail: "openai/gpt-5.5" },
          specialist: { id: "core", label: "Core", persona: "" },
          uptimeSeconds: 0,
        },
      ],
    });

    render(createElement(ConfigDialog));
    for (const select of screen.getAllByRole("combobox")) {
      expect(select).toBeDisabled();
    }
    for (const checkbox of screen.getAllByRole("checkbox")) {
      expect(checkbox).toBeDisabled();
    }
    expect(
      screen.getByText(/model and chat preference updates are temporarily disabled/i),
    ).toBeInTheDocument();
  });
});
