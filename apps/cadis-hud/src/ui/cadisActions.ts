import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  THEMES,
  normalizeAvatarStyle,
  normalizeAgentName,
  useHud,
  type AvatarStyle,
  type AgentLive,
  type AgentSessionRecord,
  type AgentSessionStatus,
  type ThemeKey,
  type VoiceDaemonStatus,
  type VoiceDiagnosticCheck,
  type VoiceDoctorReport,
  type WorkerArtifactInfo,
  type WorkerRecord,
  type WorkerStatus,
  type WorkerWorktreeInfo,
} from "./hudState.js";
import { AGENT_ROSTER } from "../lib/agents-roster.js";
import type { VoicePrefs } from "../lib/voice/voices.js";

const CLIENT_ID = "cadis-hud";
const PROTOCOL_VERSION = "0.1";
const SOCKET_PATH_STORAGE_KEY = "cadis.socketPath";
const CADIS_FRAME_EVENT = "cadis-frame";
const CADIS_SUBSCRIPTION_CLOSED_EVENT = "cadis-subscription-closed";
const FALLBACK_MAIN_MODEL = "openai/gpt-5.5";
const AGENT_ROSTER_BY_ID = new Map(AGENT_ROSTER.map((agent) => [agent.id, agent]));

type CadisRequest = {
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  client_id: typeof CLIENT_ID;
  type: string;
  payload: Record<string, unknown>;
};

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

type CadisSubscriptionClosed = {
  generation?: unknown;
  error?: unknown;
};

type RawModelDescriptor = {
  provider?: unknown;
  model?: unknown;
  provider_id?: unknown;
  model_id?: unknown;
  id?: unknown;
  name?: unknown;
  display_name?: unknown;
};

const streamingBySession = new Map<string, { id: string; text: string }>();
let connected = false;
let requestSeq = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectAttempt = 0;
let intentionalDisconnect = false;
let generation = 0;
let lastEventId: string | null = null;
let unlistenCadisFrame: (() => void) | null = null;
let unlistenCadisSubscriptionClosed: (() => void) | null = null;

export function connect(): void {
  const activeGeneration = ++generation;
  intentionalDisconnect = false;
  clearReconnect();
  useHud.getState().setGateway("connecting");

  void (async () => {
    const subscribed = await startEventSubscription(activeGeneration);
    if (!subscribed || activeGeneration !== generation) {
      scheduleReconnect();
      return;
    }

    await Promise.all([
      callCadis("models.list", {}, activeGeneration),
      callCadis("daemon.status", {}, activeGeneration),
      callCadis("voice.status", {}, activeGeneration),
    ]);
  })();
}

export function disconnect(): void {
  intentionalDisconnect = true;
  connected = false;
  generation += 1;
  clearReconnect();
  streamingBySession.clear();
  stopEventSubscription();
  useHud.getState().setGateway("disconnected");
}

export function sendUserMessage(text: string, _model?: string): boolean {
  if (!connected) return false;
  const targetAgentId = parseMentionTargetAgentId(text);
  const payload: Record<string, unknown> = {
    session_id: null,
    content: text,
    content_kind: "chat",
  };
  if (targetAgentId) payload.target_agent_id = targetAgentId;
  void callCadis("message.send", payload).then((ok) => {
    if (!ok) {
      pushSystem("(CADIS request failed - message could not be delivered)");
      scheduleReconnect();
    }
  });
  return true;
}

export function sendAgentModelUpdate(agentId: string, model: string): boolean {
  if (!connected) return false;
  void callCadis("agent.model.set", {
    agent_id: agentId,
    model,
  }).then((ok) => {
    if (!ok) scheduleReconnect();
  });
  return true;
}

export function sendAgentRename(agentId: string, name: string): boolean {
  if (!connected) return false;
  void callCadis("agent.rename", {
    agent_id: agentId,
    display_name: name,
  }).then((ok) => {
    if (!ok) scheduleReconnect();
  });
  return true;
}

export function sendApprovalResponse(id: string, verdict: "approve" | "deny"): boolean {
  if (!connected) return false;
  void callCadis("approval.respond", {
    approval_id: id,
    decision: verdict === "approve" ? "approved" : "denied",
    reason: "",
  }).then((ok) => {
    if (!ok) scheduleReconnect();
  });
  return true;
}

export function sendUiPreferencesPatch(patch: Record<string, unknown>): boolean {
  if (!connected) return false;
  void callCadis("ui.preferences.set", { patch }).then((ok) => {
    if (!ok) scheduleReconnect();
  });
  return true;
}

export function persistThemePreference(theme: ThemeKey): void {
  sendUiPreferencesPatch({ hud: { theme } });
}

export function persistBackgroundOpacityPreference(backgroundOpacity: number): void {
  sendUiPreferencesPatch({ hud: { background_opacity: backgroundOpacity } });
}

export function persistAvatarStylePreference(avatarStyle: AvatarStyle): void {
  sendUiPreferencesPatch({ hud: { avatar_style: avatarStyle } });
}

export function persistAlwaysOnTopPreference(alwaysOnTop: boolean): void {
  sendUiPreferencesPatch({ hud: { always_on_top: alwaysOnTop } });
}

export function persistVoicePreferences(prefs: VoicePrefs): void {
  sendUiPreferencesPatch({
    voice: {
      enabled: true,
      voice_id: prefs.voiceId,
      rate: prefs.rate,
      pitch: prefs.pitch,
      volume: prefs.volume,
      auto_speak: prefs.autoSpeak,
    },
  });
}

export function persistChatPreferences(prefs: { thinking: boolean; fast: boolean }): void {
  sendUiPreferencesPatch({ chat: prefs });
}

export function requestVoiceDoctor(): boolean {
  if (!connected) return false;
  void callCadis("voice.doctor", { include_bridge: true }).then((ok) => {
    if (!ok) scheduleReconnect();
  });
  return true;
}

export function sendVoicePreflight(report: VoiceDoctorReport, surface = "cadis-hud"): boolean {
  if (!connected) return false;
  void callCadis("voice.preflight", {
    surface,
    summary: report.summary,
    checks: report.checks.map((check) => ({
      name: check.name,
      status: protocolVoiceCheckStatus(check.status),
      message: check.detail,
    })),
  }).then((ok) => {
    if (!ok) scheduleReconnect();
  });
  return true;
}

export function _resetCadisActionsForTest(): void {
  disconnect();
  intentionalDisconnect = false;
  reconnectAttempt = 0;
  lastEventId = null;
}

export function _emitCadisSubscriptionFrameForTest(frame: CadisFrame): void {
  handleSubscriptionFrame(frame);
}

async function callCadis(
  type: string,
  payload: Record<string, unknown> = {},
  activeGeneration = generation,
): Promise<boolean> {
  try {
    const frames = await requestCadis(type, payload);
    if (activeGeneration !== generation) return false;
    handleFrames(frames);
    markConnected();
    return true;
  } catch {
    if (activeGeneration === generation) markDisconnected();
    return false;
  }
}

async function requestCadis(type: string, payload: Record<string, unknown>): Promise<CadisFrame[]> {
  const request = buildRequest(type, payload);
  const socketPath = readSocketPath();
  const args = socketPath ? { request, socketPath } : { request };
  const frames = await invoke<unknown>("cadis_request", args);
  return Array.isArray(frames) ? (frames as CadisFrame[]) : [];
}

function buildRequest(type: string, payload: Record<string, unknown>): CadisRequest {
  requestSeq += 1;
  return {
    protocol_version: PROTOCOL_VERSION,
    request_id: `hud-${Date.now()}-${requestSeq}`,
    client_id: CLIENT_ID,
    type,
    payload,
  };
}

async function startEventSubscription(activeGeneration: number): Promise<boolean> {
  try {
    await ensureEventListeners();
    const request = buildRequest("events.subscribe", buildSubscriptionPayload());
    const socketPath = readSocketPath();
    const args = socketPath ? { request, socketPath } : { request };
    await invoke("cadis_events_subscribe", args);
    if (activeGeneration !== generation) return false;
    markConnected();
    return true;
  } catch {
    if (activeGeneration === generation) markDisconnected();
    return false;
  }
}

function buildSubscriptionPayload(): Record<string, unknown> {
  const payload: Record<string, unknown> = {
    replay_limit: 128,
    include_snapshot: true,
  };
  if (lastEventId) payload.since_event_id = lastEventId;
  return payload;
}

async function ensureEventListeners(): Promise<void> {
  if (!unlistenCadisFrame) {
    unlistenCadisFrame = await listen<CadisFrame>(CADIS_FRAME_EVENT, (event) => {
      if (intentionalDisconnect) return;
      handleSubscriptionFrame(event.payload);
    });
  }
  if (!unlistenCadisSubscriptionClosed) {
    unlistenCadisSubscriptionClosed = await listen<CadisSubscriptionClosed>(
      CADIS_SUBSCRIPTION_CLOSED_EVENT,
      (event) => {
        if (intentionalDisconnect) return;
        const payload = asRecord(event.payload);
        const error = stringFrom(payload.error);
        generation += 1;
        markDisconnected();
        if (error) pushSystem(`(CADIS event stream ended: ${error})`);
        scheduleReconnect();
      },
    );
  }
}

function stopEventSubscription(): void {
  const frameUnlisten = unlistenCadisFrame;
  const closedUnlisten = unlistenCadisSubscriptionClosed;
  unlistenCadisFrame = null;
  unlistenCadisSubscriptionClosed = null;
  frameUnlisten?.();
  closedUnlisten?.();
  void Promise.resolve(invoke("cadis_events_unsubscribe")).catch(() => undefined);
}

function handleSubscriptionFrame(frame: CadisFrame): void {
  handleFrame(frame);
  markConnected();
}

function readSocketPath(): string | undefined {
  const envPath = (import.meta as unknown as { env?: Record<string, string | undefined> }).env
    ?.VITE_CADIS_SOCKET_PATH;
  if (envPath?.trim()) return envPath.trim();
  try {
    return localStorage.getItem(SOCKET_PATH_STORAGE_KEY)?.trim() || undefined;
  } catch {
    return undefined;
  }
}

function handleFrames(frames: CadisFrame[]): void {
  for (const frame of frames) handleFrame(frame);
}

export function handleCadisFrameForTest(frame: CadisFrame): void {
  handleFrame(frame);
}

function handleFrame(frame: CadisFrame): void {
  const envelope = unwrapEnvelope(frame);
  if (!envelope || typeof envelope.type !== "string") return;
  if (typeof envelope.event_id === "string" && envelope.event_id) lastEventId = envelope.event_id;
  handleMessage(envelope.type, envelope.payload, readSessionId(envelope));
}

function unwrapEnvelope(frame: CadisFrame): CadisEnvelope | null {
  if (frame && typeof frame === "object" && typeof frame.type === "string") {
    return frame as CadisEnvelope;
  }
  const payload = frame.payload;
  if (payload && typeof payload === "object") return payload as CadisEnvelope;
  return null;
}

function handleMessage(type: string, payload: unknown, sessionId?: string): void {
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
}

function handleDaemonStatus(payload: unknown): void {
  const p = asRecord(payload);
  const modelProvider = stringFrom(p.model_provider);
  const uptimeSeconds = numberFrom(p.uptime_seconds) ?? 0;
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

function handleModelsList(payload: unknown): void {
  const p = asRecord(payload);
  const models = Array.isArray(p.models) ? normalizeModels(p.models) : [];
  const defaultModel = models[0] ?? useHud.getState().defaultModel ?? FALLBACK_MAIN_MODEL;
  useHud.getState().setAvailableModels(models, defaultModel);
}

function handlePreferences(payload: unknown): void {
  const envelope = asRecord(payload);
  const preferences = asRecord(envelope.preferences ?? payload);
  const hud = asRecord(preferences.hud);
  const voice = asRecord(preferences.voice);
  const chat = asRecord(preferences.chat);

  const theme = stringFrom(hud.theme);
  if (isThemeKey(theme)) useHud.getState().setTheme(theme);

  const avatarStyle = normalizeAvatarStyle(stringFrom(hud.avatar_style));
  if (avatarStyle) useHud.getState().setAvatarStyle(avatarStyle);

  const opacity = numberFrom(hud.background_opacity);
  if (opacity !== undefined) useHud.getState().setBackgroundOpacity(opacity);

  const voicePatch: Partial<VoicePrefs> = {};
  const voiceId = stringFrom(voice.voice_id);
  if (voiceId) voicePatch.voiceId = voiceId;
  const rate = numberFrom(voice.rate);
  const pitch = numberFrom(voice.pitch);
  const volume = numberFrom(voice.volume);
  if (rate !== undefined) voicePatch.rate = rate;
  if (pitch !== undefined) voicePatch.pitch = pitch;
  if (volume !== undefined) voicePatch.volume = volume;
  if (typeof voice.auto_speak === "boolean") voicePatch.autoSpeak = voice.auto_speak;
  if (Object.keys(voicePatch).length) useHud.getState().updateVoicePrefs(voicePatch);

  const chatPatch: { thinking?: boolean; fast?: boolean } = {};
  if (typeof chat.thinking === "boolean") chatPatch.thinking = chat.thinking;
  if (typeof chat.fast === "boolean") chatPatch.fast = chat.fast;
  if (Object.keys(chatPatch).length) useHud.getState().setChatPreferences(chatPatch);
}

function handleSessionStarted(payload: unknown, sessionId?: string): void {
  const p = asRecord(payload);
  const sid = stringFrom(p.session_id) ?? sessionId;
  const title = stringFrom(p.title);
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

function handleVoiceStatus(payload: unknown): void {
  const status = normalizeVoiceStatus(payload);
  if (status) useHud.getState().setVoiceStatus(status);
}

function handleVoiceDoctor(payload: unknown): void {
  const p = asRecord(payload);
  const status = normalizeVoiceStatus(p.status);
  if (status) useHud.getState().setVoiceStatus(status);

  const checks = Array.isArray(p.checks)
    ? p.checks
        .map(normalizeVoiceCheck)
        .filter((check): check is VoiceDiagnosticCheck => Boolean(check))
    : [];
  useHud.getState().setVoiceDoctor({
    summary: voiceDoctorSummary(checks),
    checks,
  });
}

function handleMessageDelta(payload: unknown, sessionId?: string): void {
  const p = asRecord(payload);
  const delta = stringFrom(p.delta);
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
    agentId: stringFrom(p.agent_id),
    agentName: stringFrom(p.agent_name),
  });
}

function handleMessageCompleted(payload: unknown, sessionId?: string): void {
  const p = asRecord(payload);
  const sid = sessionId ?? "main";
  const stream = streamingBySession.get(sid);
  const finalText = stringFrom(p.content) ?? stringFrom(p.text) ?? stream?.text;
  if (!finalText) return;
  useHud.getState().upsertChat({
    id: stream?.id ?? `m-${Date.now()}-${sid}`,
    who: "cadis",
    text: finalText,
    ts: Date.now(),
    final: true,
    agentId: stringFrom(p.agent_id),
    agentName: stringFrom(p.agent_name),
  });
  useHud.getState().setVoiceState("idle");
  streamingBySession.delete(sid);
}

function handleAgentList(payload: unknown): void {
  const p = asRecord(payload);
  const agents = Array.isArray(p.agents) ? p.agents : [];
  for (const agent of agents) handleAgentSpawned(agent);
}

function handleAgentSpawned(payload: unknown): void {
  const p = asRecord(payload);
  const agentId = stringFrom(p.agent_id) ?? stringFrom(p.id);
  if (!agentId) return;

  const model = stringFrom(p.model);
  const displayName = stringFrom(p.display_name) ?? stringFrom(p.name);
  const role = stringFrom(p.role);
  const parentAgentId = stringFrom(p.parent_agent_id);
  const status = normalizeAgentStatus(stringFrom(p.status)) ?? "idle";

  upsertDaemonAgent({ agentId, displayName, role, parentAgentId, model, status });
  if (model) useHud.getState().setAgentModel(agentId, model);
}

function handleAgentRenamed(payload: unknown): void {
  const p = asRecord(payload);
  const agentId = stringFrom(p.agent_id);
  const displayName = stringFrom(p.display_name);
  if (agentId && displayName) useHud.getState().renameAgent(agentId, displayName);
}

function handleAgentModelChanged(payload: unknown): void {
  const p = asRecord(payload);
  const agentId = stringFrom(p.agent_id);
  const model = stringFrom(p.model);
  if (agentId && model) useHud.getState().setAgentModel(agentId, model);
}

function handleAgentStatusChanged(payload: unknown): void {
  const p = asRecord(payload);
  const agentId = stringFrom(p.agent_id);
  if (!agentId) return;

  const status = normalizeAgentStatus(stringFrom(p.status));
  if (status) useHud.getState().setAgentStatus(agentId, status);

  const task = stringFrom(p.task);
  if (task) {
    useHud.getState().setAgentTask(agentId, {
      verb: status === "working" ? "working" : "ready",
      target: agentId === "main" ? "session" : `${agentId} agent`,
      detail: task,
    });
  }
}

function handleAgentSessionEvent(type: string, payload: unknown, sessionId?: string): void {
  const p = asRecord(payload);
  const sessionRecordId = stringFrom(p.agent_session_id) ?? stringFrom(p.id);
  const agentId = stringFrom(p.agent_id);
  if (!sessionRecordId || !agentId) return;

  const existing = useHud
    .getState()
    .agentSessions.find((candidate) => candidate.id === sessionRecordId);
  const status =
    normalizeAgentSessionStatus(stringFrom(p.status)) ?? defaultAgentSessionStatus(type);
  const task = stringFrom(p.task) ?? existing?.task ?? "agent session";
  const budgetSteps = numberFrom(p.budget_steps) ?? existing?.budgetSteps ?? 0;
  const stepsUsed = numberFrom(p.steps_used) ?? existing?.stepsUsed ?? 0;
  const result = stringFrom(p.result);
  const error = stringFrom(p.error) ?? stringFrom(p.error_code);
  const parentAgentId = stringFrom(p.parent_agent_id) ?? existing?.parentAgentId;

  ensureKnownAgent(agentId);
  useHud.getState().upsertAgentSession({
    id: sessionRecordId,
    sessionId: stringFrom(p.session_id) ?? sessionId ?? existing?.sessionId ?? "main",
    routeId: stringFrom(p.route_id) ?? existing?.routeId ?? "",
    agentId,
    parentAgentId,
    task,
    status,
    timeoutAt: stringFrom(p.timeout_at) ?? existing?.timeoutAt,
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

function handleApprovalRequested(payload: unknown): void {
  const p = asRecord(payload);
  const approvalId = stringFrom(p.approval_id) ?? stringFrom(p.id);
  if (!approvalId) return;
  useHud.getState().pushApproval({
    id: approvalId,
    ruleId: stringFrom(p.risk_class) ?? stringFrom(p.rule_id) ?? "approval",
    reason: stringFrom(p.summary) ?? stringFrom(p.reason) ?? stringFrom(p.title) ?? "",
    cmd: stringFrom(p.command) ?? stringFrom(p.cmd) ?? "",
    cwd: stringFrom(p.workspace) ?? stringFrom(p.cwd) ?? "",
    agentId: stringFrom(p.agent_id) ?? "main",
    ts: Date.now(),
  });
}

function handleApprovalResolved(payload: unknown): void {
  const p = asRecord(payload);
  const approvalId = stringFrom(p.approval_id) ?? stringFrom(p.id);
  if (approvalId) useHud.getState().removeApproval(approvalId);
}

function handleOrchestratorRoute(payload: unknown): void {
  const p = asRecord(payload);
  const targetAgentId = stringFrom(p.target_agent_id) ?? stringFrom(p.target);
  const targetAgentName = stringFrom(p.target_agent_name) ?? agentDisplayName(targetAgentId);
  if (targetAgentId && !targetAgentName) ensureKnownAgent(targetAgentId);

  const source = stringFrom(p.source) ?? "orchestrator";
  const target = targetAgentName ?? targetAgentId ?? "agent";
  const reason = stringFrom(p.reason);
  const routeId = stringFrom(p.id) ?? `route-${Date.now()}`;
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

function handleWorkerEvent(type: string, payload: unknown): void {
  const p = asRecord(payload);
  const workerId = stringFrom(p.worker_id) ?? stringFrom(p.id) ?? stringFrom(p.agent_id);
  if (!workerId) return;
  const existing = useHud.getState().workers.find((worker) => worker.id === workerId);
  const agentId = stringFrom(p.agent_id);
  const parentAgentId = stringFrom(p.parent_agent_id) ?? agentId ?? existing?.parentAgentId ?? "main";
  const status = normalizeWorkerStatus(stringFrom(p.status)) ?? defaultWorkerStatus(type);
  const summary = stringFrom(p.summary);
  const delta = stringFrom(p.delta);
  const text = delta ?? summary ?? stringFrom(p.text) ?? existing?.lastText;
  const logTail = nextWorkerLogTail(existing, delta);
  const logLineCount = existing ? existing.logLineCount + (delta ? 1 : 0) : delta ? 1 : 0;
  ensureKnownAgent(parentAgentId);
  useHud.getState().upsertWorker({
    id: workerId,
    agentId: agentId ?? existing?.agentId,
    parentAgentId,
    cli: stringFrom(p.cli) ?? existing?.cli,
    cwd: stringFrom(p.cwd) ?? existing?.cwd,
    status,
    lastText: text,
    summary: summary ?? existing?.summary,
    logLineCount,
    logTail,
    worktree: readWorkerWorktree(p.worktree) ?? existing?.worktree,
    artifacts: readWorkerArtifacts(p.artifacts) ?? existing?.artifacts,
    startedAt: numberFrom(p.started_at) ?? existing?.startedAt ?? Date.now(),
    updatedAt: numberFrom(p.updated_at) ?? Date.now(),
  });
}

function handleRequestRejected(payload: unknown): void {
  const p = asRecord(payload);
  const message = stringFrom(p.message) ?? "CADIS request was rejected";
  pushSystem(`(${message})`);
}

function parseMentionTargetAgentId(text: string): string | undefined {
  const match = text.match(/^@([A-Za-z0-9._-]+)(?:\s+|$)/);
  const token = match?.[1];
  if (!token) return undefined;
  return resolveMentionTargetAgentId(token);
}

function resolveMentionTargetAgentId(token: string): string {
  const target = normalizeMentionToken(token);
  const known = useHud.getState().agents.find((agent) => {
    const names = [agent.spec.id, agent.spec.name, agent.spec.role];
    return names.some((name) => normalizeMentionToken(name) === target);
  });
  return known?.spec.id ?? token;
}

function normalizeMentionToken(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "");
}

function agentDisplayName(agentId: string | undefined): string | undefined {
  if (!agentId) return undefined;
  return useHud.getState().agents.find((agent) => agent.spec.id === agentId)?.spec.name;
}

function upsertDaemonAgent({
  agentId,
  displayName,
  role,
  parentAgentId,
  model,
  status,
}: {
  agentId: string;
  displayName?: string;
  role?: string;
  parentAgentId?: string;
  model?: string;
  status: AgentLive["status"];
}): void {
  const existing = useHud.getState().agents.find((agent) => agent.spec.id === agentId);
  const rosterSpec = AGENT_ROSTER_BY_ID.get(agentId);
  const baseSpec = existing?.spec ?? rosterSpec ?? {
    id: agentId,
    name: agentId,
    role: "Agent",
    icon: "◈",
    hue: deterministicHue(agentId),
    tasks: [],
  };
  const name = normalizeAgentName(displayName ?? baseSpec.name, baseSpec.name);
  const nextRole = normalizeAgentName(role ?? baseSpec.role, baseSpec.role);
  upsertAgent({
    spec: {
      ...baseSpec,
      id: agentId,
      name,
      role: nextRole,
    },
    status,
    currentTask: {
      verb: status === "working" ? "working" : "ready",
      target: parentAgentId ? `child of ${parentAgentId}` : `${name} agent`,
      detail: model ?? existing?.currentTask.detail ?? FALLBACK_MAIN_MODEL,
    },
    uptimeSeconds: existing?.uptimeSeconds ?? 0,
    parentAgentId,
  });
}

function deterministicHue(value: string): number {
  let hash = 0;
  for (const char of value) hash = (hash * 31 + char.charCodeAt(0)) % 360;
  return hash;
}

function upsertAgent(agent: AgentLive): void {
  useHud.setState((state) => {
    const index = state.agents.findIndex((candidate) => candidate.spec.id === agent.spec.id);
    if (index === -1) return { agents: [...state.agents, agent] };
    const next = [...state.agents];
    next[index] = { ...next[index]!, ...agent };
    return { agents: next };
  });
}

function normalizeModels(models: unknown[]): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const model of models) {
    const normalized = coerceModel(model);
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    out.push(normalized);
  }
  return out;
}

function coerceModel(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (!value || typeof value !== "object") return null;
  const v = value as RawModelDescriptor;
  if (typeof v.provider === "string" && typeof v.model === "string") return joinModel(v.provider, v.model);
  if (typeof v.provider_id === "string" && typeof v.model_id === "string") {
    return joinModel(v.provider_id, v.model_id);
  }
  if (typeof v.id === "string") return v.id;
  if (typeof v.model === "string") return v.model;
  if (typeof v.model_id === "string") return v.model_id;
  if (typeof v.name === "string") return v.name;
  if (typeof v.display_name === "string") return v.display_name;
  return null;
}

function joinModel(provider: string, model: string): string {
  const cleanProvider = provider.trim();
  const cleanModel = model.trim();
  if (!cleanProvider) return cleanModel;
  if (!cleanModel) return cleanProvider;
  return cleanModel.includes("/") ? cleanModel : `${cleanProvider}/${cleanModel}`;
}

function normalizeAgentStatus(status: string | undefined): AgentLive["status"] | null {
  if (!status) return null;
  const normalized = status.toLowerCase();
  if (normalized === "spawning" || normalized === "starting" || normalized === "started") {
    return "waiting";
  }
  if (normalized === "running" || normalized === "working") return "working";
  if (normalized === "waitingapproval" || normalized === "waiting_approval" || normalized === "waiting") {
    return "waiting";
  }
  if (normalized === "idle" || normalized === "completed") return "idle";
  if (normalized === "failed" || normalized === "error" || normalized === "timed_out") return "idle";
  if (normalized === "cancelled" || normalized === "canceled") return "idle";
  return null;
}

function normalizeAgentSessionStatus(status: string | undefined): AgentSessionStatus | null {
  if (!status) return null;
  const normalized = status.toLowerCase();
  if (normalized === "started" || normalized === "starting") return "started";
  if (normalized === "running" || normalized === "working") return "running";
  if (normalized === "completed" || normalized === "complete" || normalized === "succeeded") {
    return "completed";
  }
  if (normalized === "failed" || normalized === "error") return "failed";
  if (normalized === "cancelled" || normalized === "canceled") return "cancelled";
  if (normalized === "timed_out" || normalized === "timeout") return "timed_out";
  if (normalized === "budget_exceeded" || normalized === "budgetexceeded") {
    return "budget_exceeded";
  }
  return null;
}

function normalizeWorkerStatus(status: string | undefined): WorkerStatus | null {
  if (!status) return null;
  const normalized = status.toLowerCase();
  if (normalized === "spawning" || normalized === "starting" || normalized === "started") {
    return "spawning";
  }
  if (normalized === "running" || normalized === "working") return "running";
  if (normalized === "completed" || normalized === "complete" || normalized === "succeeded") {
    return "completed";
  }
  if (normalized === "failed" || normalized === "error") return "failed";
  if (normalized === "cancelled" || normalized === "canceled") return "cancelled";
  return null;
}

function normalizeVoiceStatus(payload: unknown): VoiceDaemonStatus | null {
  const p = asRecord(payload);
  const provider = stringFrom(p.provider) ?? "edge";
  const voiceId = stringFrom(p.voice_id) ?? "id-ID-GadisNeural";
  const sttLanguage = stringFrom(p.stt_language) ?? "auto";
  const maxSpokenChars = numberFrom(p.max_spoken_chars) ?? 800;
  const bridge = stringFrom(p.bridge) ?? "hud-local";
  const state = normalizeVoiceState(stringFrom(p.state));
  const lastPreflight = asRecord(p.last_preflight);
  const surface = stringFrom(lastPreflight.surface);
  const checkedAt = stringFrom(lastPreflight.checked_at);
  const summary = stringFrom(lastPreflight.summary);
  const preflightStatus = stringFrom(lastPreflight.status);

  return {
    enabled: p.enabled === true,
    state,
    provider,
    voiceId,
    sttLanguage,
    maxSpokenChars,
    bridge,
    ...(surface && checkedAt && summary && preflightStatus
      ? {
          lastPreflight: {
            surface,
            checkedAt,
            summary,
            status: preflightStatus,
          },
        }
      : {}),
  };
}

function normalizeVoiceState(state: string | undefined): VoiceDaemonStatus["state"] {
  if (
    state === "disabled" ||
    state === "ready" ||
    state === "degraded" ||
    state === "blocked" ||
    state === "unknown"
  ) {
    return state;
  }
  return "unknown";
}

function normalizeVoiceCheck(value: unknown): VoiceDiagnosticCheck | null {
  const p = asRecord(value);
  const name = stringFrom(p.name);
  if (!name) return null;
  return {
    name,
    status: hudVoiceCheckStatus(stringFrom(p.status)),
    detail: stringFrom(p.message) ?? stringFrom(p.detail) ?? "",
  };
}

function hudVoiceCheckStatus(status: string | undefined): VoiceDiagnosticCheck["status"] {
  const normalized = status?.toLowerCase();
  if (normalized === "ok" || normalized === "pass" || normalized === "ready") return "pass";
  if (normalized === "error" || normalized === "fail" || normalized === "blocked") return "fail";
  return "warn";
}

function protocolVoiceCheckStatus(status: VoiceDiagnosticCheck["status"]): string {
  if (status === "pass") return "ok";
  if (status === "fail") return "error";
  return "warn";
}

function voiceDoctorSummary(checks: VoiceDiagnosticCheck[]): string {
  const failures = checks.filter((check) => check.status === "fail").length;
  const warnings = checks.filter((check) => check.status === "warn").length;
  if (failures) return `${failures} blocking issue${failures === 1 ? "" : "s"}`;
  if (warnings) return `${warnings} warning${warnings === 1 ? "" : "s"}`;
  return checks.length ? "ready" : "not run";
}

function defaultWorkerStatus(type: string): WorkerStatus {
  if (type === "worker.started") return "spawning";
  if (type === "worker.completed") return "completed";
  if (type === "worker.failed") return "failed";
  if (type === "worker.cancelled") return "cancelled";
  return "running";
}

function defaultAgentSessionStatus(type: string): AgentSessionStatus {
  if (type === "agent.session.started") return "started";
  if (type === "agent.session.completed") return "completed";
  if (type === "agent.session.failed") return "failed";
  if (type === "agent.session.cancelled") return "cancelled";
  return "running";
}

function agentStatusFromSession(status: AgentSessionStatus): AgentLive["status"] {
  if (status === "started" || status === "running") return "working";
  if (status === "failed" || status === "timed_out" || status === "budget_exceeded") return "idle";
  if (status === "cancelled") return "idle";
  return "idle";
}

function agentTaskVerbFromSession(status: AgentSessionStatus): string {
  if (status === "completed") return "completed";
  if (status === "failed" || status === "timed_out" || status === "budget_exceeded") return "failed";
  if (status === "cancelled") return "cancelled";
  return "working";
}

function agentSessionDetail({
  status,
  budgetSteps,
  stepsUsed,
  result,
  error,
}: Pick<AgentSessionRecord, "status" | "budgetSteps" | "stepsUsed" | "result" | "error">): string {
  if (status === "completed" && result) return result;
  if ((status === "failed" || status === "timed_out" || status === "budget_exceeded") && error) {
    return error;
  }
  if (budgetSteps > 0) return `step ${Math.min(stepsUsed, budgetSteps)}/${budgetSteps}`;
  return status.replace("_", " ");
}

function nextWorkerLogTail(existing: WorkerRecord | undefined, delta: string | undefined): string[] {
  if (!delta) return existing?.logTail ?? [];
  return [...(existing?.logTail ?? []), delta].slice(-3);
}

function readWorkerWorktree(value: unknown): WorkerWorktreeInfo | undefined {
  const worktree = asRecord(value);
  const state = stringFrom(worktree.state);
  const branchName = stringFrom(worktree.branch_name);
  const worktreePath = stringFrom(worktree.worktree_path);
  const cleanupPolicy = stringFrom(worktree.cleanup_policy);
  if (!state && !branchName && !worktreePath && !cleanupPolicy) return undefined;
  return { state, branchName, worktreePath, cleanupPolicy };
}

function readWorkerArtifacts(value: unknown): WorkerArtifactInfo | undefined {
  const artifacts = asRecord(value);
  const summary = stringFrom(artifacts.summary);
  const patch = stringFrom(artifacts.patch);
  const testReport = stringFrom(artifacts.test_report);
  const testReportStatus =
    stringFrom(artifacts.test_report_status) ??
    stringFrom(artifacts.test_status) ??
    stringFrom(artifacts.tests_status);
  if (!summary && !patch && !testReport && !testReportStatus) return undefined;
  return { summary, patch, testReport, testReportStatus };
}

function readSessionId(envelope: CadisEnvelope): string | undefined {
  if (typeof envelope.session_id === "string" && envelope.session_id) return envelope.session_id;
  const payload = asRecord(envelope.payload);
  return stringFrom(payload.session_id);
}

function markConnected(): void {
  connected = true;
  reconnectAttempt = 0;
  useHud.getState().setGateway("connected");
}

function markDisconnected(): void {
  connected = false;
  useHud.getState().setGateway("disconnected");
}

function scheduleReconnect(): void {
  if (intentionalDisconnect || reconnectTimer) return;
  const delay = computeBackoffMs(reconnectAttempt);
  reconnectAttempt = Math.min(reconnectAttempt + 1, 10);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect();
  }, delay);
}

export function computeBackoffMs(attempt: number, rand: () => number = Math.random): number {
  const safeAttempt = Math.max(0, Math.min(10, attempt));
  const base = Math.min(30_000, 1_000 * 2 ** safeAttempt);
  const jitter = Math.floor(rand() * 400) - 200;
  return Math.max(500, base + jitter);
}

function clearReconnect(): void {
  if (!reconnectTimer) return;
  clearTimeout(reconnectTimer);
  reconnectTimer = null;
}

function pushSystem(text: string): void {
  useHud.getState().pushChat({
    id: `m-${Date.now()}-system`,
    who: "system",
    text,
    ts: Date.now(),
  });
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function stringFrom(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function numberFrom(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function isThemeKey(value: string | undefined): value is ThemeKey {
  return Boolean(value && value in THEMES);
}

export function ensureKnownAgent(agentId: string, model?: string): void {
  if (useHud.getState().agents.some((agent) => agent.spec.id === agentId)) return;
  upsertDaemonAgent({ agentId, model, status: "idle" });
}
