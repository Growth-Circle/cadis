import { beforeEach, describe, expect, it } from "vitest";
import { computeBackoffMs, handleCadisFrameForTest } from "./cadisActions.js";
import { useHud } from "./hudState.js";

beforeEach(() => {
  useHud.setState({
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

  it("computes bounded reconnect backoff", () => {
    expect(computeBackoffMs(0, () => 0.5)).toBe(1_000);
    expect(computeBackoffMs(20, () => 0.5)).toBe(30_000);
    expect(computeBackoffMs(0, () => 0)).toBeGreaterThanOrEqual(500);
  });
});
