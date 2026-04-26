import "@testing-library/jest-dom/vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_ROSTER } from "../../lib/agents-roster.js";
import { DEFAULT_VOICE_PREFS } from "../../lib/voice/voices.js";
import {
  available as sttAvailable,
  startListening,
  type SttDebugSnapshot,
  type SttHandlers,
} from "../../lib/voice/stt.js";
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
  vi.mocked(sttAvailable).mockReturnValue(false);
  vi.mocked(startListening).mockReset();
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

describe("ChatPanel voice UX", () => {
  it("keeps daemon-aligned auto-speak disabled by default", () => {
    expect(DEFAULT_VOICE_PREFS.autoSpeak).toBe(false);
  });

  it("shows empty STT transcript status and opens mic debug when audio was heard", async () => {
    let handlers: SttHandlers | undefined;
    vi.mocked(sttAvailable).mockReturnValue(true);
    vi.mocked(startListening).mockImplementation((_lang, nextHandlers) => {
      handlers = nextHandlers;
      return { stop: vi.fn() };
    });

    render(<ChatPanel />);

    fireEvent.click(screen.getByRole("button", { name: "microphone" }));
    await waitFor(() => expect(handlers).toBeDefined());
    await act(async () => {
      handlers?.onDebug?.(debugSnapshot({
        stage: "done",
        message: "whisper returned empty text",
        language: "id",
        elapsedMs: 1800,
        level: 0,
        rms: 0.01,
        peak: 0.08,
        voiceDetected: true,
        silentMs: 1800,
        chunks: 4,
        bytes: 2048,
        pcmFrames: 8,
        pcmBytes: 4096,
        captureSource: "webaudio-pcm+mediarecorder",
        permissionState: "granted",
        deviceCount: 1,
        deviceLabels: "Test mic",
        selectedDeviceLabel: "Test mic",
        streamActive: true,
        trackLabel: "Test mic",
        trackEnabled: true,
        trackMuted: false,
        trackReadyState: "live",
        recorderState: "inactive",
        mimeType: "audio/webm",
        audioContextState: "running",
        sampleRate: 48000,
        analyserFrames: 12,
        silenceReason: "voice ended after trailing silence",
        stopReason: "silence",
        transcript: "",
        error: "",
      }));
      handlers?.onEmpty?.({
        message: "audio was heard, but whisper returned no transcript",
        audioHeard: true,
        stopReason: "silence",
      });
      handlers?.onEnd?.();
    });

    await waitFor(() => {
      expect(screen.getByText("(stt status: audio was heard, but whisper returned no transcript)")).toBeInTheDocument();
    });
    expect(screen.getByText("mic debug")).toBeInTheDocument();
    expect(screen.getByText("whisper returned empty text")).toBeInTheDocument();
    expect(screen.getAllByText("Test mic").length).toBeGreaterThan(0);
    expect(screen.getByText("webaudio-pcm+mediarecorder")).toBeInTheDocument();
    expect(screen.getByText("voice ended after trailing silence")).toBeInTheDocument();
  });

  it("starts deterministic mic debug capture without submitting audio for transcription", async () => {
    let handlers: SttHandlers | undefined;
    vi.mocked(sttAvailable).mockReturnValue(true);
    vi.mocked(startListening).mockImplementation((_lang, nextHandlers) => {
      handlers = nextHandlers;
      return { stop: vi.fn() };
    });

    render(<ChatPanel />);

    fireEvent.click(screen.getByRole("button", { name: "debug mic" }));
    await waitFor(() => expect(handlers).toBeDefined());

    expect(startListening).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        onDebug: expect.any(Function),
        onLevel: expect.any(Function),
      }),
      { debugOnly: true },
    );

    await act(async () => {
      handlers?.onLevel?.({
        level: 0.4,
        rms: 0.009,
        peak: 0.06,
        samples: Array.from({ length: 48 }, (_, index) => (index % 4) / 4),
      });
      handlers?.onDebug?.(debugSnapshot({
        stage: "recording",
        message: "voice signal detected",
        level: 0.4,
        rms: 0.009,
        peak: 0.06,
        voiceDetected: true,
        pcmFrames: 5,
        pcmBytes: 4096,
        captureSource: "webaudio-pcm",
        permissionState: "granted",
        deviceCount: 2,
        deviceLabels: "Built-in Audio, USB Mic",
        selectedDeviceLabel: "USB Mic",
        streamActive: true,
        trackReadyState: "live",
        trackSampleRate: 48000,
        trackChannelCount: 1,
        analyserFrames: 5,
        silenceReason: "trailing silence after voice",
      }));
    });

    expect(screen.getByText("mic debug capture")).toBeInTheDocument();
    expect(screen.getByText("USB Mic")).toBeInTheDocument();
    expect(screen.getByText("webaudio-pcm")).toBeInTheDocument();
    expect(screen.getByText("Built-in Audio, USB Mic")).toBeInTheDocument();
    expect(screen.getByText("trailing silence after voice")).toBeInTheDocument();
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

function debugSnapshot(overrides: Partial<SttDebugSnapshot>): SttDebugSnapshot {
  return {
    stage: "idle",
    message: "idle",
    language: "auto",
    elapsedMs: 0,
    level: 0,
    rms: 0,
    peak: 0,
    samples: [],
    voiceDetected: false,
    silentMs: 0,
    chunks: 0,
    bytes: 0,
    pcmFrames: 0,
    pcmBytes: 0,
    captureSource: "",
    permissionState: "",
    deviceCount: 0,
    deviceLabels: "",
    selectedDeviceId: "",
    selectedDeviceLabel: "",
    streamActive: false,
    streamId: "",
    trackLabel: "",
    trackEnabled: false,
    trackMuted: false,
    trackReadyState: "",
    trackDeviceId: "",
    trackGroupId: "",
    trackSampleRate: 0,
    trackChannelCount: 0,
    recorderState: "",
    mimeType: "",
    audioContextState: "",
    sampleRate: 0,
    analyserFftSize: 0,
    analyserFrames: 0,
    silenceReason: "",
    stopReason: "",
    transcript: "",
    error: "",
    ...overrides,
  };
}
