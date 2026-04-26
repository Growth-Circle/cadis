export type MockCadisFrame = {
  frame: "event";
  payload: {
    protocol_version?: string;
    event_id: string;
    timestamp: string;
    source: "cadisd";
    session_id?: string;
    type: string;
    payload: Record<string, unknown>;
  };
};

export const mockCadisDaemonWorkerStream: MockCadisFrame[] = [
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_001",
      timestamp: "2026-04-26T00:00:00Z",
      source: "cadisd",
      type: "daemon.status",
      payload: { state: "connected", version: "0.1.0", model_provider: "echo", uptime_seconds: 4 },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_002",
      timestamp: "2026-04-26T00:00:01Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "agent.spawned",
      payload: {
        agent_id: "codex",
        display_name: "Codex",
        role: "Coding",
        parent_agent_id: "main",
        model: "codex-cli/chatgpt-plan",
        status: "running",
      },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_003",
      timestamp: "2026-04-26T00:00:02Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "agent.session.started",
      payload: {
        agent_session_id: "ags_mock_001",
        session_id: "ses_mock_1",
        route_id: "route_mock_001",
        agent_id: "codex",
        parent_agent_id: "main",
        task: "run focused HUD worker tests",
        status: "running",
        timeout_at: "2026-04-26T00:15:00Z",
        budget_steps: 3,
        steps_used: 0,
      },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_004",
      timestamp: "2026-04-26T00:00:03Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "worker.started",
      payload: {
        worker_id: "worker_mock_001",
        agent_id: "codex",
        parent_agent_id: "main",
        status: "running",
        summary: "Worker Coding: run focused HUD worker tests",
        worktree: {
          worktree_root: ".cadis/worktrees",
          worktree_path: ".cadis/worktrees/worker_mock_001",
          branch_name: "cadis/worker_mock_001/hud-worker-progress",
          state: "planned",
          cleanup_policy: "explicit",
        },
        artifacts: {
          root: "/home/user/.cadis/artifacts/workers/worker_mock_001",
          patch: "/home/user/.cadis/artifacts/workers/worker_mock_001/patch.diff",
          test_report: "/home/user/.cadis/artifacts/workers/worker_mock_001/test-report.json",
          summary: "/home/user/.cadis/artifacts/workers/worker_mock_001/summary.md",
          changed_files: "/home/user/.cadis/artifacts/workers/worker_mock_001/changed-files.json",
          memory_candidates: "/home/user/.cadis/artifacts/workers/worker_mock_001/memory-candidates.jsonl",
        },
      },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_005",
      timestamp: "2026-04-26T00:00:04Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "worker.log.delta",
      payload: {
        worker_id: "worker_mock_001",
        agent_id: "codex",
        parent_agent_id: "main",
        delta: "started: Worker Coding: run focused HUD worker tests\n",
      },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_006",
      timestamp: "2026-04-26T00:00:05Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "agent.session.updated",
      payload: {
        agent_session_id: "ags_mock_001",
        session_id: "ses_mock_1",
        route_id: "route_mock_001",
        agent_id: "codex",
        parent_agent_id: "main",
        task: "run focused HUD worker tests",
        status: "running",
        timeout_at: "2026-04-26T00:15:00Z",
        budget_steps: 3,
        steps_used: 2,
      },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_007",
      timestamp: "2026-04-26T00:00:06Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "worker.completed",
      payload: {
        worker_id: "worker_mock_001",
        agent_id: "codex",
        parent_agent_id: "main",
        status: "completed",
        summary: "completed: focused HUD worker tests passed",
      },
    },
  },
  {
    frame: "event",
    payload: {
      protocol_version: "0.1",
      event_id: "evt_mock_008",
      timestamp: "2026-04-26T00:00:07Z",
      source: "cadisd",
      session_id: "ses_mock_1",
      type: "agent.session.completed",
      payload: {
        agent_session_id: "ags_mock_001",
        session_id: "ses_mock_1",
        route_id: "route_mock_001",
        agent_id: "codex",
        parent_agent_id: "main",
        task: "run focused HUD worker tests",
        status: "completed",
        timeout_at: "2026-04-26T00:15:00Z",
        budget_steps: 3,
        steps_used: 3,
        result: "focused HUD worker tests passed",
      },
    },
  },
];
