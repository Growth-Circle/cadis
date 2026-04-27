import "@testing-library/jest-dom/vitest";
import { act, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_ROSTER } from "../lib/agents-roster.js";
import { DEFAULT_VOICE_PREFS } from "../lib/voice/voices.js";
import { ChatPanel } from "./chat/ChatPanel.js";
import { handleCadisFrameForTest, _resetCadisActionsForTest } from "./cadisActions.js";
import { useHud, type AgentLive } from "./hudState.js";
import { OrbitalHUD } from "./orbital/OrbitalHUD.js";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(vi.fn())),
}));

vi.mock("../lib/voice/tts.js", () => ({
  speak: vi.fn(() => Promise.resolve()),
  stopSpeaking: vi.fn(() => Promise.resolve()),
}));

vi.mock("../lib/voice/stt.js", () => ({
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
  _resetCadisActionsForTest();
  useHud.setState({
    gateway: "connected",
    agents: INITIAL_AGENTS,
    agentModels: INITIAL_AGENT_MODELS,
    availableModels: ["openai/gpt-5.5"],
    defaultModel: "openai/gpt-5.5",
    chat: [],
    approvals: [],
    workers: [],
    selectedWorkerId: null,
    codeWorkPanelOpen: false,
    avatarStyle: "orb",
    voiceState: "idle",
    voicePrefs: DEFAULT_VOICE_PREFS,
  });
});

describe("HUD live progress acceptance", () => {
  it("renders session, route, agent status, and streamed deltas before completion", () => {
    render(
      <>
        <OrbitalHUD />
        <ChatPanel />
      </>,
    );

    emitLiveEvent("evt_001", "session.started", {
      session_id: "ses_track_a",
      title: "Track A HUD acceptance",
    });
    emitLiveEvent("evt_002", "orchestrator.route", {
      id: "route_track_a",
      source: "hud-chat",
      target_agent_id: "codex",
      target_agent_name: "Codex",
      reason: "@codex prefix",
    });
    emitLiveEvent("evt_003", "agent.status.changed", {
      agent_id: "codex",
      status: "working",
      task: "generating visible streamed answer",
    });
    emitLiveEvent("evt_004", "message.delta", {
      session_id: "ses_track_a",
      delta: "Reading live daemon progress",
      content_kind: "chat",
      agent_id: "codex",
      agent_name: "Codex",
    });

    expect(screen.getByText("(session started: Track A HUD acceptance)")).toBeVisible();
    expect(screen.getByText("(route: hud-chat -> Codex; @codex prefix)")).toBeVisible();
    expect(screen.getByText("Reading live daemon progress")).toBeVisible();
    expect(screen.queryByText("Final answer after provider completion.")).not.toBeInTheDocument();

    const codexWidget = screen.getByText("Codex").closest(".agent-widget");
    expect(codexWidget).not.toBeNull();
    expect(within(codexWidget as HTMLElement).getAllByText("working")[0]).toBeVisible();
    expect(within(codexWidget as HTMLElement).getByText("generating visible streamed answer")).toBeVisible();
    expect(useHud.getState().chat.find((message) => message.who === "cadis")).toMatchObject({
      text: "Reading live daemon progress",
      final: false,
      agentId: "codex",
      agentName: "Codex",
    });

    emitLiveEvent("evt_005", "message.completed", {
      session_id: "ses_track_a",
      content_kind: "chat",
      content: "Final answer after provider completion.",
      agent_id: "codex",
      agent_name: "Codex",
    });

    expect(screen.getByText("Final answer after provider completion.")).toBeVisible();
    expect(useHud.getState().chat.find((message) => message.who === "cadis")).toMatchObject({
      text: "Final answer after provider completion.",
      final: true,
    });
  });
});

function emitLiveEvent(eventId: string, type: string, payload: Record<string, unknown>): void {
  act(() => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        event_id: eventId,
        session_id: payload.session_id,
        type,
        payload,
      },
    });
  });
}
