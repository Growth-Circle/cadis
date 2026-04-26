import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  _resetCadisActionsForTest,
  computeBackoffMs,
  connect,
  handleCadisFrameForTest,
  sendUserMessage,
} from "./cadisActions.js";
import { useHud } from "./hudState.js";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const INITIAL_AGENTS = useHud.getState().agents;
const INITIAL_AGENT_MODELS = useHud.getState().agentModels;
const INITIAL_AVAILABLE_MODELS = useHud.getState().availableModels;
const INITIAL_DEFAULT_MODEL = useHud.getState().defaultModel;

beforeEach(() => {
  _resetCadisActionsForTest();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
  useHud.setState({
    agents: INITIAL_AGENTS,
    agentModels: INITIAL_AGENT_MODELS,
    availableModels: INITIAL_AVAILABLE_MODELS,
    defaultModel: INITIAL_DEFAULT_MODEL,
    chat: [],
    approvals: [],
    workers: [],
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
          role: "Coding",
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
      },
    ]);
  });

  it("computes bounded reconnect backoff", () => {
    expect(computeBackoffMs(0, () => 0.5)).toBe(1_000);
    expect(computeBackoffMs(20, () => 0.5)).toBe(30_000);
    expect(computeBackoffMs(0, () => 0)).toBeGreaterThanOrEqual(500);
  });
});

function sentRequest(): { type: string; payload: Record<string, unknown> } {
  const args = invokeMock.mock.calls[0]?.[1] as
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
