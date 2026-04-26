import { invoke } from "@tauri-apps/api/core";
import {
  THEMES,
  useHud,
  type AgentLive,
  type ThemeKey,
  type WorkerStatus,
} from "./hudState.js";
import { AGENT_ROSTER } from "../lib/agents-roster.js";
import type { VoicePrefs } from "../lib/voice/voices.js";

const CLIENT_ID = "cadis-hud";
const PROTOCOL_VERSION = "0.1";
const SOCKET_PATH_STORAGE_KEY = "cadis.socketPath";
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
};

type CadisFrame = {
  frame?: unknown;
  payload?: unknown;
  type?: unknown;
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

export function connect(): void {
  const activeGeneration = ++generation;
  intentionalDisconnect = false;
  clearReconnect();
  useHud.getState().setGateway("connecting");

  void (async () => {
    const ok = await callCadis("daemon.status", {}, activeGeneration);
    if (!ok || activeGeneration !== generation) {
      scheduleReconnect();
      return;
    }

    await Promise.all([
      callCadis("models.list", {}, activeGeneration),
      callCadis("ui.preferences.get", {}, activeGeneration),
    ]);
  })();
}

export function disconnect(): void {
  intentionalDisconnect = true;
  connected = false;
  generation += 1;
  clearReconnect();
  streamingBySession.clear();
  useHud.getState().setGateway("disconnected");
}

export function sendUserMessage(text: string, _model?: string): boolean {
  if (!connected) return false;
  void callCadis("message.send", {
    session_id: null,
    content: text,
    content_kind: "chat",
  }).then((ok) => {
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

export function _resetCadisActionsForTest(): void {
  disconnect();
  intentionalDisconnect = false;
  reconnectAttempt = 0;
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
  if (type === "daemon.status.response") {
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
  if (type === "message.delta") {
    handleMessageDelta(payload, sessionId);
    return;
  }
  if (type === "message.completed") {
    handleMessageCompleted(payload, sessionId);
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
  if (type === "approval.requested") {
    handleApprovalRequested(payload);
    return;
  }
  if (type === "approval.resolved") {
    handleApprovalResolved(payload);
    return;
  }
  if (type === "worker.started" || type === "worker.log.delta" || type === "worker.completed") {
    handleWorkerEvent(type, payload);
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

function handleWorkerEvent(type: string, payload: unknown): void {
  const p = asRecord(payload);
  const workerId = stringFrom(p.worker_id);
  if (!workerId) return;
  const status: WorkerStatus = type === "worker.completed" ? "completed" : "running";
  useHud.getState().upsertWorker({
    id: workerId,
    parentAgentId: stringFrom(p.agent_id) ?? "main",
    status,
    lastText: stringFrom(p.delta) ?? stringFrom(p.summary),
    startedAt: Date.now(),
    updatedAt: Date.now(),
  });
}

function handleRequestRejected(payload: unknown): void {
  const p = asRecord(payload);
  const message = stringFrom(p.message) ?? "CADIS request was rejected";
  pushSystem(`(${message})`);
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

function normalizeAgentStatus(status: string | undefined): "working" | "idle" | "waiting" | null {
  if (!status) return null;
  const normalized = status.toLowerCase();
  if (normalized === "running" || normalized === "working") return "working";
  if (normalized === "waitingapproval" || normalized === "waiting_approval" || normalized === "waiting") {
    return "waiting";
  }
  if (normalized === "idle" || normalized === "completed" || normalized === "failed") return "idle";
  return null;
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
  const spec = AGENT_ROSTER_BY_ID.get(agentId) ?? {
    id: agentId,
    name: agentId,
    role: "Agent",
    icon: "◈",
    hue: 210,
    tasks: [],
  };
  upsertAgent({
    spec,
    status: "idle",
    currentTask: {
      verb: "ready",
      target: `${spec.name} agent`,
      detail: model ?? FALLBACK_MAIN_MODEL,
    },
    uptimeSeconds: 0,
  });
}
