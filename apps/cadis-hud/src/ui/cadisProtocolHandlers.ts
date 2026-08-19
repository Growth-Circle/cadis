import {
  normalizeAvatarStyle,
  useHud,
  type VoiceDiagnosticCheck,
} from "./hudState.js";
import type { VoicePrefs } from "../lib/voice/voices.js";
import {
  normalizeModels,
  normalizeAgentStatus,
  normalizeAgentSessionStatus,
  normalizeWorkerStatus,
  normalizeVoiceStatus,
  normalizeVoiceCheck,
  voiceDoctorSummary,
  defaultWorkerStatus,
  defaultAgentSessionStatus,
  agentStatusFromSession,
  agentTaskVerbFromSession,
  agentSessionDetail,
  nextWorkerLogTail,
  readWorkerWorktree,
  readWorkerArtifacts,
  readAgentSpecialist,
  readSessionId,
  upsertDaemonAgent,
  agentDisplayName,
  ensureKnownAgent,
  pushSystem,
  isThemeKey,
  FALLBACK_MAIN_MODEL,
} from "./cadisNormalizers.js";
import {
  validateSafe,
  handleValidationFailure,
  DaemonStatusSchema,
  ModelsListSchema,
  PreferencesSchema,
  SessionStartedSchema,
  MessageDeltaSchema,
  MessageCompletedSchema,
  VoiceStatusSchema,
  VoiceDoctorSchema,
  AgentSpawnedSchema,
  AgentRenamedSchema,
  AgentModelChangedSchema,
  AgentSpecialistChangedSchema,
  AgentStatusChangedSchema,
  AgentListSchema,
  AgentSessionEventSchema,
  ApprovalRequestedSchema,
  ApprovalResolvedSchema,
  OrchestratorRouteSchema,
  WorkerEventSchema,
  RequestRejectedSchema,
  PatchCreatedSchema,
  TestResultSchema,
} from "./cadisProtocolSchemas.js";

type CadisEnvelope = {
  type?: unknown;
  payload?: unknown;
  session_id?: unknown;
  event_id?: unknown;
};

type CadisFrame = {
  frame?: unknown;
  payload?: unknown;
  type?: unknown;
};

/** Module-level streaming state shared with connection layer via setter. */
let streamingBySession: Map<string, { id: string; text: string }>;

export function _setStreamingBySession(map: Map<string, { id: string; text: string }>): void {
  streamingBySession = map;
}

/** Module-level lastEventId shared with connection layer via getter/setter. */
let lastEventId: string | null = null;
const processedEventIds = new Set<string>();
const processedEventOrder: string[] = [];
const MAX_PROCESSED_EVENT_IDS = 2_048;

export function _getLastEventId(): string | null {
  return lastEventId;
}

export function _setLastEventId(value: string | null): void {
  lastEventId = value;
}

export function _resetProcessedEventIds(): void {
  processedEventIds.clear();
  processedEventOrder.length = 0;
}

export function handleFrames(frames: CadisFrame[]): void {
  for (const frame of frames) handleFrame(frame);
}

export function handleCadisFrameForTest(frame: CadisFrame): void {
  handleFrame(frame);
}

export function handleFrame(frame: CadisFrame): void {
  const envelope = unwrapEnvelope(frame);
  if (!envelope || typeof envelope.type !== "string") return;
  if (typeof envelope.event_id === "string" && envelope.event_id) {
    if (processedEventIds.has(envelope.event_id)) return;
    rememberProcessedEventId(envelope.event_id);
    lastEventId = envelope.event_id;
  }
  handleMessage(envelope.type, envelope.payload, readSessionId(envelope));
}

function rememberProcessedEventId(eventId: string): void {
  processedEventIds.add(eventId);
  processedEventOrder.push(eventId);
  while (processedEventOrder.length > MAX_PROCESSED_EVENT_IDS) {
    const stale = processedEventOrder.shift();
    if (stale) processedEventIds.delete(stale);
  }
}

export function unwrapEnvelope(frame: CadisFrame): CadisEnvelope | null {
  if (frame && typeof frame === "object" && typeof frame.type === "string") {
    return frame as CadisEnvelope;
  }
  const payload = frame.payload;
  if (payload && typeof payload === "object") return payload as CadisEnvelope;
  return null;
}

export function handleMessage(type: string, payload: unknown, sessionId?: string): void {
  if (type === "request.accepted") return;
  if (type === "request.rejected") {
    handleRequestRejected(payload);
    return;
  }
  if (type === "daemon.status.response" || type === "daemon.status") {
    handleDaemonStatus(payload);
    return;
  }
  if (type === "models.list.response") {
    handleModelsList(payload);
    return;
  }
  if (type === "ui.preferences.updated") {
    handlePreferences(payload);
    return;
  }
  if (type === "session.started") {
    handleSessionStarted(payload, sessionId);
    return;
  }
  if (type === "voice.status.updated") {
    handleVoiceStatus(payload);
    return;
  }
  if (type === "voice.doctor.response" || type === "voice.preflight.response") {
    handleVoiceDoctor(payload);
    return;
  }
  if (type === "voice.preview.started" || type === "voice.started") {
    useHud.getState().setVoiceState("speaking");
    return;
  }
  if (
    type === "voice.preview.completed" ||
    type === "voice.preview.failed" ||
    type === "voice.completed" ||
    type === "voice.failed"
  ) {
    useHud.getState().setVoiceState("idle");
    return;
  }
  if (type === "message.delta") {
    handleMessageDelta(payload, sessionId);
    return;
  }
  if (type === "message.completed") {
    handleMessageCompleted(payload, sessionId);
    return;
  }
  if (type === "agent.list.response") {
    handleAgentList(payload);
    return;
  }
  if (type === "agent.spawned") {
    handleAgentSpawned(payload);
    return;
  }
  if (type === "agent.renamed") {
    handleAgentRenamed(payload);
    return;
  }
  if (type === "agent.model.changed") {
    handleAgentModelChanged(payload);
    return;
  }
  if (type === "agent.specialist.changed") {
    handleAgentSpecialistChanged(payload);
    return;
  }
  if (type === "agent.status.changed") {
    handleAgentStatusChanged(payload);
    return;
  }
  if (
    type === "agent.session.started" ||
    type === "agent.session.updated" ||
    type === "agent.session.completed" ||
    type === "agent.session.failed" ||
    type === "agent.session.cancelled"
  ) {
    handleAgentSessionEvent(type, payload, sessionId);
    return;
  }
  if (type === "approval.requested") {
    handleApprovalRequested(payload);
    return;
  }
  if (type === "approval.resolved") {
    handleApprovalResolved(payload);
    return;
  }
  if (type === "orchestrator.route") {
    handleOrchestratorRoute(payload);
    return;
  }
  if (
    type === "worker.started" ||
    type === "worker.log.delta" ||
    type === "worker.completed" ||
    type === "worker.failed" ||
    type === "worker.cancelled" ||
    type === "worker.event"
  ) {
    handleWorkerEvent(type, payload);
    return;
  }
  if (type === "patch.created") {
    handlePatchCreated(payload);
    return;
  }
  if (type === "test.result") {
    handleTestResult(payload);
    return;
  }
}

export function handleDaemonStatus(payload: unknown): void {
  const validated = validateSafe(DaemonStatusSchema, payload, "daemon.status");
  if (!validated) return;

  const modelProvider = validated.model_provider;
  const uptimeSeconds = validated.uptime_seconds ?? 0;
  useHud.getState().setAgentTask("main", {
    verb: "ready",
    target: "CADIS daemon",
    detail: modelProvider ? `provider ${modelProvider}` : "connected",
  });
  useHud.setState((state) => ({
    agents: state.agents.map((agent) =>
      agent.spec.id === "main" ? { ...agent, status: "idle", uptimeSeconds } : agent,
    ),
  }));
}

export function handleModelsList(payload: unknown): void {
  const validated = validateSafe(ModelsListSchema, payload, "models.list");
  if (!validated) return;

  const models = validated.models ? normalizeModels(validated.models) : [];
  const defaultModel = models[0] ?? useHud.getState().defaultModel ?? FALLBACK_MAIN_MODEL;
  useHud.getState().setAvailableModels(models, defaultModel);
}

export function handlePreferences(payload: unknown): void {
  const validated = validateSafe(PreferencesSchema, payload, "ui.preferences");
  if (!validated) return;

  const preferences = validated.preferences ?? {};
  const hud = preferences.hud ?? {};
  const voice = preferences.voice ?? {};
  const chat = preferences.chat ?? {};

  const theme = hud.theme;
  if (theme && isThemeKey(theme)) useHud.getState().setTheme(theme);

  const avatarStyle = normalizeAvatarStyle(hud.avatar_style);
  if (avatarStyle) useHud.getState().setAvatarStyle(avatarStyle);

  const opacity = hud.background_opacity;
  if (opacity !== undefined) useHud.getState().setBackgroundOpacity(opacity);

  const voicePatch: Partial<VoicePrefs> = {};
  if (typeof voice.enabled === "boolean") voicePatch.enabled = voice.enabled;
  if (voice.provider && (voice.provider === "edge" || voice.provider === "elevenlabs")) {
    voicePatch.provider = voice.provider;
  }
  if (voice.voice_id) voicePatch.voiceId = voice.voice_id;
  if (voice.rate !== undefined) voicePatch.rate = voice.rate;
  if (voice.pitch !== undefined) voicePatch.pitch = voice.pitch;
  if (voice.volume !== undefined) voicePatch.volume = voice.volume;
  if (typeof voice.auto_speak === "boolean") voicePatch.autoSpeak = voice.auto_speak;
  if (Object.keys(voicePatch).length) useHud.getState().updateVoicePrefs(voicePatch);

  const chatPatch: { thinking?: boolean; fast?: boolean } = {};
  if (typeof chat.thinking === "boolean") chatPatch.thinking = chat.thinking;
  if (typeof chat.fast === "boolean") chatPatch.fast = chat.fast;
  if (Object.keys(chatPatch).length) useHud.getState().setChatPreferences(chatPatch);
}

export function handleSessionStarted(payload: unknown, sessionId?: string): void {
  const validated = validateSafe(SessionStartedSchema, payload, "session.started");
  if (!validated) return;

  const sid = validated.session_id ?? sessionId;
  const title = validated.title;
  const label = title ?? sid;
  if (!label) return;
  useHud.getState().upsertChat({
    id: `session-${sid ?? label}`,
    who: "system",
    text: `(session started: ${label})`,
    ts: Date.now(),
    final: true,
  });
}

export function handleVoiceStatus(payload: unknown): void {
  const validated = validateSafe(VoiceStatusSchema, payload, "voice.status");
  if (!validated) return;

  const status = normalizeVoiceStatus(validated);
  if (status) useHud.getState().setVoiceStatus(status);
}

export function handleVoiceDoctor(payload: unknown): void {
  const validated = validateSafe(VoiceDoctorSchema, payload, "voice.doctor");
  if (!validated) return;

  const status = normalizeVoiceStatus(validated.status);
  if (status) useHud.getState().setVoiceStatus(status);

  const checks = validated.checks
    ? validated.checks
        .map(normalizeVoiceCheck)
        .filter((check): check is VoiceDiagnosticCheck => Boolean(check))
    : [];
  useHud.getState().setVoiceDoctor({
    summary: voiceDoctorSummary(checks),
    checks,
  });
}

export function handleMessageDelta(payload: unknown, sessionId?: string): void {
  const validated = validateSafe(
    MessageDeltaSchema,
    payload,
    "message.delta",
    (error) => handleValidationFailure("message.delta", error, true), // Notify user for message errors
  );
  if (!validated) return;

  const delta = validated.delta;
  if (!delta) return;
  const sid = sessionId ?? "main";
  const stream = streamingBySession.get(sid) ?? { id: `m-${Date.now()}-${sid}`, text: "" };
  stream.text += delta;
  streamingBySession.set(sid, stream);
  useHud.getState().upsertChat({
    id: stream.id,
    who: "cadis",
    text: stream.text,
    ts: Date.now(),
    final: false,
    agentId: validated.agent_id,
    agentName: validated.agent_name,
  });
}

export function handleMessageCompleted(payload: unknown, sessionId?: string): void {
  const validated = validateSafe(
    MessageCompletedSchema,
    payload,
    "message.completed",
    (error) => handleValidationFailure("message.completed", error, true), // Notify user for message errors
  );
  if (!validated) return;

  const sid = sessionId ?? "main";
  const stream = streamingBySession.get(sid);
  const finalText = validated.content ?? validated.text ?? stream?.text;
  if (!finalText) return;
  useHud.getState().upsertChat({
    id: stream?.id ?? `m-${Date.now()}-${sid}`,
    who: "cadis",
    text: finalText,
    ts: Date.now(),
    final: true,
    agentId: validated.agent_id,
    agentName: validated.agent_name,
  });
  useHud.getState().setVoiceState("idle");
  streamingBySession.delete(sid);
}

export function handleAgentList(payload: unknown): void {
  const validated = validateSafe(AgentListSchema, payload, "agent.list");
  if (!validated) return;

  const agents = validated.agents ?? [];
  for (const agent of agents) handleAgentSpawned(agent);
}

export function handleAgentSpawned(payload: unknown): void {
  const validated = validateSafe(AgentSpawnedSchema, payload, "agent.spawned");
  if (!validated) return;

  const agentId = validated.agent_id ?? validated.id;
  if (!agentId) return;

  const model = validated.model;
  const displayName = validated.display_name ?? validated.name;
  const role = validated.role;
  const parentAgentId = validated.parent_agent_id;
  const status = normalizeAgentStatus(validated.status) ?? "idle";
  const specialist = readAgentSpecialist(validated as Record<string, unknown>, role);

  upsertDaemonAgent({ agentId, displayName, role, parentAgentId, model, status, specialist });
  if (model) useHud.getState().setAgentModel(agentId, model);
}

export function handleAgentRenamed(payload: unknown): void {
  const validated = validateSafe(AgentRenamedSchema, payload, "agent.renamed");
  if (!validated) return;

  const agentId = validated.agent_id;
  const displayName = validated.display_name;
  if (agentId && displayName) useHud.getState().renameAgent(agentId, displayName);
}

export function handleAgentModelChanged(payload: unknown): void {
  const validated = validateSafe(AgentModelChangedSchema, payload, "agent.model.changed");
  if (!validated) return;

  const agentId = validated.agent_id;
  const model = validated.model;
  if (agentId && model) useHud.getState().setAgentModel(agentId, model);
}

export function handleAgentSpecialistChanged(payload: unknown): void {
  const validated = validateSafe(AgentSpecialistChangedSchema, payload, "agent.specialist.changed");
  if (!validated) return;

  const agentId = validated.agent_id;
  if (!agentId) return;
  const existing = useHud.getState().agents.find((agent) => agent.spec.id === agentId);
  const specialist = readAgentSpecialist(validated as Record<string, unknown>, existing?.spec.role);
  if (specialist) useHud.getState().setAgentSpecialist(agentId, specialist);
}

export function handleAgentStatusChanged(payload: unknown): void {
  const validated = validateSafe(AgentStatusChangedSchema, payload, "agent.status.changed");
  if (!validated) return;

  const agentId = validated.agent_id;
  if (!agentId) return;

  const status = normalizeAgentStatus(validated.status);
  if (status) useHud.getState().setAgentStatus(agentId, status);

  const task = validated.task;
  if (task) {
    useHud.getState().setAgentTask(agentId, {
      verb: status === "working" ? "working" : "ready",
      target: agentId === "main" ? "session" : `${agentId} agent`,
      detail: task,
    });
  }
}

export function handleAgentSessionEvent(type: string, payload: unknown, sessionId?: string): void {
  const validated = validateSafe(AgentSessionEventSchema, payload, `agent.session.${type}`);
  if (!validated) return;

  const sessionRecordId = validated.agent_session_id ?? validated.id;
  const agentId = validated.agent_id;
  if (!sessionRecordId || !agentId) return;

  const existing = useHud
    .getState()
    .agentSessions.find((candidate) => candidate.id === sessionRecordId);
  const status =
    normalizeAgentSessionStatus(validated.status) ?? defaultAgentSessionStatus(type);
  const task = validated.task ?? existing?.task ?? "agent session";
  const budgetSteps = validated.budget_steps ?? existing?.budgetSteps ?? 0;
  const stepsUsed = validated.steps_used ?? existing?.stepsUsed ?? 0;
  const result = validated.result;
  const error = validated.error ?? validated.error_code;
  const parentAgentId = validated.parent_agent_id ?? existing?.parentAgentId;

  ensureKnownAgent(agentId);
  useHud.getState().upsertAgentSession({
    id: sessionRecordId,
    sessionId: validated.session_id ?? sessionId ?? existing?.sessionId ?? "main",
    routeId: validated.route_id ?? existing?.routeId ?? "",
    agentId,
    parentAgentId,
    task,
    status,
    timeoutAt: validated.timeout_at?.toString() ?? existing?.timeoutAt,
    budgetSteps,
    stepsUsed,
    result,
    error,
    updatedAt: Date.now(),
  });

  useHud.getState().setAgentStatus(agentId, agentStatusFromSession(status));
  useHud.getState().setAgentTask(agentId, {
    verb: agentTaskVerbFromSession(status),
    target: task,
    detail: agentSessionDetail({ status, budgetSteps, stepsUsed, result, error }),
  });
}

export function handleApprovalRequested(payload: unknown): void {
  const validated = validateSafe(
    ApprovalRequestedSchema,
    payload,
    "approval.requested",
    (error) => handleValidationFailure("approval.requested", error, true), // Critical: notify user
  );
  if (!validated) return;

  const approvalId = validated.approval_id ?? validated.id;
  if (!approvalId) return;
  useHud.getState().pushApproval({
    id: approvalId,
    ruleId: validated.risk_class ?? validated.rule_id ?? "approval",
    reason: validated.summary ?? validated.reason ?? validated.title ?? "",
    cmd: validated.command ?? validated.cmd ?? "",
    cwd: validated.workspace ?? validated.cwd ?? "",
    agentId: validated.agent_id ?? "main",
    ts: Date.now(),
  });
}

export function handleApprovalResolved(payload: unknown): void {
  const validated = validateSafe(ApprovalResolvedSchema, payload, "approval.resolved");
  if (!validated) return;

  const approvalId = validated.approval_id ?? validated.id;
  if (approvalId) useHud.getState().removeApproval(approvalId);
}

export function handleOrchestratorRoute(payload: unknown): void {
  const validated = validateSafe(OrchestratorRouteSchema, payload, "orchestrator.route");
  if (!validated) return;

  const targetAgentId = validated.target_agent_id ?? validated.target;
  const targetAgentName = validated.target_agent_name ?? agentDisplayName(targetAgentId);
  if (targetAgentId && !targetAgentName) ensureKnownAgent(targetAgentId);

  const source = validated.source ?? "orchestrator";
  const target = targetAgentName ?? targetAgentId ?? "agent";
  const reason = validated.reason;
  const routeId = validated.id ?? `route-${Date.now()}`;
  useHud.getState().upsertChat({
    id: `route-${routeId}`,
    who: "system",
    text: reason ? `(route: ${source} -> ${target}; ${reason})` : `(route: ${source} -> ${target})`,
    ts: Date.now(),
    agentId: targetAgentId,
    agentName: targetAgentName,
    final: true,
  });
}

export function handleWorkerEvent(type: string, payload: unknown): void {
  const validated = validateSafe(WorkerEventSchema, payload, `worker.${type}`);
  if (!validated) return;

  const workerId = validated.worker_id ?? validated.id ?? validated.agent_id;
  if (!workerId) return;
  const existing = useHud.getState().workers.find((worker) => worker.id === workerId);
  const agentId = validated.agent_id;
  const parentAgentId = validated.parent_agent_id ?? agentId ?? existing?.parentAgentId ?? "main";
  const status = normalizeWorkerStatus(validated.status) ?? defaultWorkerStatus(type);
  const summary = validated.summary;
  const delta = validated.delta;
  const text = delta ?? summary ?? validated.text ?? existing?.lastText;
  const logTail = nextWorkerLogTail(existing, delta);
  const logLineCount = existing ? existing.logLineCount + (delta ? 1 : 0) : delta ? 1 : 0;
  ensureKnownAgent(parentAgentId);
  useHud.getState().upsertWorker({
    id: workerId,
    agentId: agentId ?? existing?.agentId,
    parentAgentId,
    cli: validated.cli ?? existing?.cli,
    cwd: validated.cwd ?? existing?.cwd,
    status,
    lastText: text,
    summary: summary ?? existing?.summary,
    logLineCount,
    logTail,
    worktree: readWorkerWorktree(validated.worktree) ?? existing?.worktree,
    artifacts: readWorkerArtifacts(validated.artifacts) ?? existing?.artifacts,
    startedAt: validated.started_at ?? existing?.startedAt ?? Date.now(),
    updatedAt: validated.updated_at ?? Date.now(),
  });
}

export function handleRequestRejected(payload: unknown): void {
  const validated = validateSafe(RequestRejectedSchema, payload, "request.rejected");
  if (!validated) return;

  const message = validated.message ?? "CADIS request was rejected";
  pushSystem(`(${message})`);
}

export function handlePatchCreated(payload: unknown): void {
  const validated = validateSafe(PatchCreatedSchema, payload, "patch.created");
  if (!validated) return;

  const id = validated.patch_id ?? `patch-${Date.now()}`;
  const summary = validated.summary ?? "patch created";
  useHud.getState().pushPatch({ id, summary });
}

export function handleTestResult(payload: unknown): void {
  const validated = validateSafe(TestResultSchema, payload, "test.result");
  if (!validated) return;

  const status = validated.status ?? "unknown";
  const summary = validated.summary ?? status;
  useHud.getState().pushTestResult({ id: `test-${Date.now()}`, summary: `[${status}] ${summary}` });
}
