import "@testing-library/jest-dom/vitest";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { createElement } from "react";
import { defaultSpecialistForRole } from "../../lib/agent-specialists.js";
import { useHud } from "../hudState.js";
import { AgentRenameDialog } from "./AgentRenameDialog.js";

const sendAgentRenameMock = vi.hoisted(() => vi.fn(() => true));
const sendAgentSpecialistUpdateMock = vi.hoisted(() => vi.fn(() => true));

vi.mock("../cadisActions.js", () => ({
  sendAgentRename: sendAgentRenameMock,
  sendAgentSpecialistUpdate: sendAgentSpecialistUpdateMock,
}));

beforeEach(() => {
  sendAgentRenameMock.mockClear().mockReturnValue(true);
  sendAgentSpecialistUpdateMock.mockClear().mockReturnValue(true);
  useHud.setState({
    gateway: "connected",
    agentRenameTarget: "atlas",
    agents: [
      {
        spec: { id: "atlas", name: "Atlas", role: "Research", icon: "A", hue: 180, tasks: [] },
        status: "idle",
        currentTask: { verb: "ready", target: "Atlas agent", detail: "openai/gpt-5.5" },
        specialist: defaultSpecialistForRole("Research"),
        uptimeSeconds: 0,
      },
    ],
  });
});

describe("AgentRenameDialog", () => {
  it("sends agent.rename protocol message on save", () => {
    render(createElement(AgentRenameDialog));

    const input = screen.getByLabelText(/agent name/i);
    fireEvent.change(input, { target: { value: "NewName" } });
    fireEvent.click(screen.getByText("SAVE"));

    expect(sendAgentRenameMock).toHaveBeenCalledWith("atlas", "NewName");
  });

  it("falls back to default name when input is empty", () => {
    render(createElement(AgentRenameDialog));

    const input = screen.getByLabelText(/agent name/i);
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.click(screen.getByText("SAVE"));

    // normalizeAgentName("   ") returns "CADIS" (the default fallback)
    expect(sendAgentRenameMock).toHaveBeenCalledWith("atlas", "CADIS");
  });

  it("enforces max length 32 on the name input", () => {
    render(createElement(AgentRenameDialog));

    const input = screen.getByLabelText(/agent name/i) as HTMLInputElement;
    expect(input.maxLength).toBe(32);
  });

  it("disables save while disconnected", () => {
    useHud.setState({ gateway: "disconnected" });
    render(createElement(AgentRenameDialog));

    const save = screen.getByRole("button", { name: "SAVE" });
    expect(save).toBeDisabled();
    fireEvent.click(save);
    expect(sendAgentRenameMock).not.toHaveBeenCalled();
    expect(sendAgentSpecialistUpdateMock).not.toHaveBeenCalled();
  });
});
