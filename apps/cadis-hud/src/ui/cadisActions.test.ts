import "@testing-library/jest-dom/vitest";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { createElement } from "react";
import {
  _resetCadisActionsForTest,
  _emitCadisSubscriptionFrameForTest,
  computeBackoffMs,
  connect,
  handleCadisFrameForTest,
  sendUserMessage,
  sendVoicePreflight,
} from "./cadisActions.js";
import { useHud } from "./hudState.js";
import { WorkerTree } from "./orbital/WorkerTree.js";
import { mockCadisDaemonWorkerStream } from "./fixtures/mockCadisDaemonEventStream.js";

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

const INITIAL_AGENTS = useHud.getState().agents;
const INITIAL_AGENT_MODELS = useHud.getState().agentModels;
const INITIAL_AVAILABLE_MODELS = useHud.getState().availableModels;
const INITIAL_DEFAULT_MODEL = useHud.getState().defaultModel;

beforeEach(() => {
  _resetCadisActionsForTest();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
  listenMock.mockReset();
  listenMock.mockResolvedValue(vi.fn());
  useHud.setState({
    agents: INITIAL_AGENTS,
    agentModels: INITIAL_AGENT_MODELS,
    availableModels: INITIAL_AVAILABLE_MODELS,
    defaultModel: INITIAL_DEFAULT_MODEL,
    agentSessions: [],
    chat: [],
    approvals: [],
    workers: [],
    voiceStatus: null,
    voiceDoctor: null,
    gateway: "disconnected",
  });
});

describe("cadisActions", () => {
  it("normalizes CADIS message frames into chat state", () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "message.delta",
        session_id: "ses_1",
        payload: { delta: "Halo" },
      },
    });
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "message.completed",
        session_id: "ses_1",
        payload: {},
      },
    });

    expect(useHud.getState().chat).toMatchObject([
      { who: "cadis", text: "Halo", final: true },
    ]);
  });

  it("normalizes model descriptors from the daemon catalog", () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "models.list.response",
        payload: {
          models: [
            { provider: "codex-cli", model: "chatgpt-plan" },
            { provider: "openai", model: "gpt-5.2" },
          ],
        },
      },
    });

    expect(useHud.getState().availableModels).toEqual([
      "codex-cli/chatgpt-plan",
      "openai/gpt-5.2",
    ]);
  });

  it("routes leading @mentions by target_agent_id without rewriting content", async () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "agent.spawned",
        payload: {
          agent_id: "agent_42",
          display_name: "Builder",
          role: "Build Ops",
          parent_agent_id: "main",
          model: "openai/gpt-5.2",
        },
      },
    });
    connect();
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(4));
    invokeMock.mockClear();

    expect(sendUserMessage("@Builder run tests")).toBe(true);

    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1));
    expect(sentRequest().type).toBe("message.send");
    expect(sentRequest().payload).toMatchObject({
      content: "@Builder run tests",
      content_kind: "chat",
      target_agent_id: "agent_42",
    });
  });

  it("resolves leading @mentions by agent role before sending target_agent_id", async () => {
    connect();
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(4));
    useHud.setState({
      agents: [
        ...useHud.getState().agents,
        {
          spec: {
            id: "agent_42",
            name: "Builder",
            role: "Build Ops",
            icon: "B",
            hue: 200,
            tasks: [],
          },
          status: "idle",
          currentTask: {
            verb: "ready",
            target: "Builder agent",
            detail: "openai/gpt-5.2",
          },
          uptimeSeconds: 0,
          parentAgentId: "main",
        },
      ],
    });
    invokeMock.mockClear();

    expect(sendUserMessage("@BuildOps run tests")).toBe(true);

    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1));
    expect(sentRequest().payload).toMatchObject({
      content: "@BuildOps run tests",
      content_kind: "chat",
      target_agent_id: "agent_42",
    });
  });

  it("upserts daemon-spawned agents", () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "agent.spawned",
        payload: {
          agent_id: "agent_42",
          display_name: "Builder",
          role: "Coding",
          parent_agent_id: "main",
          model: "openai/gpt-5.2",
        },
      },
    });

    const state = useHud.getState();
    const agent = state.agents.find((candidate) => candidate.spec.id === "agent_42");
    expect(agent).toMatchObject({
      spec: { id: "agent_42", name: "Builder", role: "Coding" },
      parentAgentId: "main",
      currentTask: { detail: "openai/gpt-5.2" },
    });
    expect(state.agentModels.agent_42).toBe("openai/gpt-5.2");
  });

  it("starts an events.subscribe bridge with bounded replay from the last event id", async () => {
    _emitCadisSubscriptionFrameForTest({
      frame: "event",
      payload: {
        event_id: "evt_000120",
        type: "ui.preferences.updated",
        payload: { preferences: { hud: { theme: "ice" } } },
      },
    });

    connect();

    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(invokeMock.mock.calls[0]?.[0]).toBe("cadis_events_subscribe");
    expect(sentRequest(0)).toMatchObject({
      type: "events.subscribe",
      payload: {
        since_event_id: "evt_000120",
        replay_limit: 128,
        include_snapshot: true,
      },
    });
  });

  it("records orchestrator route events as chat rows", () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "orchestrator.route",
        payload: {
          id: "route_1",
          source: "hud-chat",
          target_agent_id: "codex",
          target_agent_name: "Codex",
          reason: "@codex prefix",
        },
      },
    });

    expect(useHud.getState().chat).toMatchObject([
      {
        id: "route-route_1",
        who: "system",
        text: "(route: hud-chat -> Codex; @codex prefix)",
        agentId: "codex",
        agentName: "Codex",
      },
    ]);
  });

  it("normalizes richer worker payload fields", () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "worker.log.delta",
        payload: {
          worker_id: "worker_1",
          agent_id: "agent_42",
          parent_agent_id: "codex",
          status: "running",
          delta: "running tests",
          cli: "codex",
          cwd: "/repo",
        },
      },
    });
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "worker.completed",
        payload: {
          worker_id: "worker_1",
          status: "completed",
          summary: "tests passed",
        },
      },
    });

    expect(useHud.getState().workers).toMatchObject([
      {
        id: "worker_1",
        agentId: "agent_42",
        parentAgentId: "codex",
        cli: "codex",
        cwd: "/repo",
        status: "completed",
        lastText: "tests passed",
        summary: "tests passed",
        logLineCount: 1,
        logTail: ["running tests"],
      },
    ]);
  });

  it("records daemon-visible voice status and doctor checks", () => {
    handleCadisFrameForTest({
      frame: "event",
      payload: {
        type: "voice.doctor.response",
        payload: {
          status: {
            enabled: true,
            state: "degraded",
            provider: "edge",
            voice_id: "id-ID-GadisNeural",
            stt_language: "auto",
            max_spoken_chars: 800,
            bridge: "hud-local",
            last_preflight: {
              surface: "cadis-hud",
              status: "warn",
              summary: "1 warning",
              checked_at: "2026-04-26T00:00:00Z",
            },
          },
          checks: [
            { name: "voice.provider", status: "ok", message: "configured provider edge" },
            { name: "microphone", status: "warn", message: "permission pending" },
          ],
        },
      },
    });

    expect(useHud.getState().voiceStatus).toMatchObject({
      enabled: true,
      state: "degraded",
      provider: "edge",
      lastPreflight: { surface: "cadis-hud", summary: "1 warning" },
    });
    expect(useHud.getState().voiceDoctor).toMatchObject({
      summary: "1 warning",
      checks: [
        { name: "voice.provider", status: "pass", detail: "configured provider edge" },
        { name: "microphone", status: "warn", detail: "permission pending" },
      ],
    });
  });

  it("publishes HUD bridge preflight checks to the daemon", async () => {
    connect();
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(4));
    invokeMock.mockClear();

    expect(
      sendVoicePreflight({
        summary: "ready",
        checks: [{ name: "microphone", status: "pass", detail: "1 input visible" }],
      }),
    ).toBe(true);

    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1));
    expect(sentRequest()).toMatchObject({
      type: "voice.preflight",
      payload: {
        surface: "cadis-hud",
        summary: "ready",
        checks: [{ name: "microphone", status: "ok", message: "1 input visible" }],
      },
    });
  });

  it("renders worker progress from a mock daemon event stream", () => {
    for (const frame of mockCadisDaemonWorkerStream) {
      handleCadisFrameForTest(frame);
    }

    const state = useHud.getState();
    expect(state.agentSessions).toMatchObject([
      {
        id: "ags_mock_001",
        agentId: "codex",
        status: "completed",
        stepsUsed: 3,
        budgetSteps: 3,
        result: "focused HUD worker tests passed",
      },
    ]);
    expect(state.workers).toMatchObject([
      {
        id: "worker_mock_001",
        agentId: "codex",
        parentAgentId: "main",
        status: "completed",
        summary: "completed: focused HUD worker tests passed",
        logLineCount: 1,
        worktree: {
          state: "planned",
          branchName: "cadis/worker_mock_001/hud-worker-progress",
        },
        artifacts: {
          summary: "/home/user/.cadis/artifacts/workers/worker_mock_001/summary.md",
          testReport: "/home/user/.cadis/artifacts/workers/worker_mock_001/test-report.json",
        },
      },
    ]);

    render(createElement(WorkerTree, { agentId: "codex" }));

    expect(screen.getByText(/workers · 1/)).toBeInTheDocument();
    expect(screen.getByText("ags_mock_001")).toBeInTheDocument();
    expect(screen.getByText("worker_mock_001")).toBeInTheDocument();
    expect(screen.getByText(/focused HUD worker tests passed/)).toBeInTheDocument();
  });

  it("computes bounded reconnect backoff", () => {
    expect(computeBackoffMs(0, () => 0.5)).toBe(1_000);
    expect(computeBackoffMs(20, () => 0.5)).toBe(30_000);
    expect(computeBackoffMs(0, () => 0)).toBeGreaterThanOrEqual(500);
  });
});

function sentRequest(index = 0): { type: string; payload: Record<string, unknown> } {
  const args = invokeMock.mock.calls[index]?.[1] as
    | { request?: { type?: unknown; payload?: unknown } }
    | undefined;
  return {
    type: typeof args?.request?.type === "string" ? args.request.type : "",
    payload:
      args?.request?.payload && typeof args.request.payload === "object"
        ? (args.request.payload as Record<string, unknown>)
        : {},
  };
}
