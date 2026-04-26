import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_ROSTER } from "../../lib/agents-roster.js";
import { DEFAULT_VOICE_PREFS } from "../../lib/voice/voices.js";
import { useHud, type AgentLive } from "../hudState.js";
import { buildMentionOptions, ChatPanel, getActiveMentionQuery } from "./ChatPanel.js";

vi.mock("../cadisActions.js", () => ({
  sendUserMessage: vi.fn(() => true),
}));

vi.mock("../../lib/voice/tts.js", () => ({
  speak: vi.fn(() => Promise.resolve()),
  stopSpeaking: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../lib/voice/stt.js", () => ({
  available: vi.fn(() => false),
  startListening: vi.fn(),
}));

const INITIAL_AGENT_MODELS = Object.fromEntries(
  AGENT_ROSTER.map((agent) => [agent.id, "openai/gpt-5.5"]),
);
const INITIAL_AGENTS: AgentLive[] = AGENT_ROSTER.map((agent) => ({
  spec: agent,
  status: agent.id === "main" ? "idle" : "waiting",
  currentTask: {
    verb: "ready",
    target: agent.id === "main" ? "CADIS agent" : `${agent.name} agent`,
    detail: "openai/gpt-5.5",
  },
  uptimeSeconds: 0,
}));

beforeEach(() => {
  useHud.setState({
    gateway: "connected",
    agents: INITIAL_AGENTS,
    agentModels: INITIAL_AGENT_MODELS,
    defaultModel: "openai/gpt-5.5",
    chat: [],
    approvals: [],
    workers: [],
    voiceState: "idle",
    voicePrefs: DEFAULT_VOICE_PREFS,
  });
});

describe("ChatPanel mention picker", () => {
  it("detects only leading active mention tokens", () => {
    expect(getActiveMentionQuery("@")).toBe("");
    expect(getActiveMentionQuery("@co")).toBe("co");
    expect(getActiveMentionQuery("@codex run tests")).toBeNull();
    expect(getActiveMentionQuery("ask @codex")).toBeNull();
  });

  it("filters agents by id, name, or role", () => {
    expect(buildMentionOptions(INITIAL_AGENTS, "co")[0]).toMatchObject({
      id: "codex",
      name: "Codex",
    });
    expect(buildMentionOptions(INITIAL_AGENTS, "sec")[0]).toMatchObject({
      id: "aegis",
      role: "Security",
    });
  });

  it("shows agent names after @ and inserts the selected handle", () => {
    render(<ChatPanel />);

    const input = screen.getByPlaceholderText("or type a command...");
    fireEvent.change(input, { target: { value: "@co" } });

    expect(screen.getByRole("listbox", { name: "agent mentions" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("option", { name: /@codex/i }));

    expect(input).toHaveValue("@codex ");
  });
});
