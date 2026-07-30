import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useHud } from "./hudState.js";
import {
  handleFrame,
  handleFrames,
  _setStreamingBySession,
  _getLastEventId,
  _setLastEventId,
  _resetProcessedEventIds,
} from "./cadisProtocolHandlers.js";
import { pushSystem, asRecord, stringFrom } from "./cadisNormalizers.js";
import { resetValidationFailureCount } from "./cadisProtocolSchemas.js";

const CLIENT_ID = "cadis-hud";
const PROTOCOL_VERSION = "0.2";
const SOCKET_PATH_STORAGE_KEY = "cadis.socketPath";
const CADIS_FRAME_EVENT = "cadis-frame";
const CADIS_SUBSCRIPTION_CLOSED_EVENT = "cadis-subscription-closed";

type CadisRequest = {
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  client_id: typeof CLIENT_ID;
  type: string;
  payload: Record<string, unknown>;
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

const streamingBySession = new Map<string, { id: string; text: string }>();
_setStreamingBySession(streamingBySession);

let connected = false;
let requestSeq = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectAttempt = 0;
let intentionalDisconnect = false;
let generation = 0;
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

export async function callCadis(
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

export async function requestCadis(type: string, payload: Record<string, unknown>): Promise<CadisFrame[]> {
  const request = buildRequest(type, payload);
  const socketPath = readSocketPath();
  const args = socketPath ? { request, socketPath } : { request };
  const frames = await invoke<unknown>("cadis_request", args);
  return Array.isArray(frames) ? (frames as CadisFrame[]) : [];
}

export function buildRequest(type: string, payload: Record<string, unknown>): CadisRequest {
  requestSeq += 1;
  return {
    protocol_version: PROTOCOL_VERSION,
    request_id: `hud-${Date.now()}-${requestSeq}`,
    client_id: CLIENT_ID,
    type,
    payload,
  };
}

export async function startEventSubscription(activeGeneration: number): Promise<boolean> {
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

export function buildSubscriptionPayload(): Record<string, unknown> {
  const payload: Record<string, unknown> = {
    replay_limit: 128,
    include_snapshot: true,
  };
  const lastId = _getLastEventId();
  if (lastId) payload.since_event_id = lastId;
  return payload;
}

export async function ensureEventListeners(): Promise<void> {
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

export function stopEventSubscription(): void {
  const frameUnlisten = unlistenCadisFrame;
  const closedUnlisten = unlistenCadisSubscriptionClosed;
  unlistenCadisFrame = null;
  unlistenCadisSubscriptionClosed = null;
  frameUnlisten?.();
  closedUnlisten?.();
  void Promise.resolve(invoke("cadis_events_unsubscribe")).catch(() => undefined);
}

export function handleSubscriptionFrame(frame: CadisFrame): void {
  handleFrame(frame);
  markConnected();
}

export function readSocketPath(): string | undefined {
  const envPath = (import.meta as unknown as { env?: Record<string, string | undefined> }).env
    ?.VITE_CADIS_SOCKET_PATH;
  if (envPath?.trim()) return envPath.trim();
  try {
    return localStorage.getItem(SOCKET_PATH_STORAGE_KEY)?.trim() || undefined;
  } catch {
    return undefined;
  }
}

export function markConnected(): void {
  connected = true;
  reconnectAttempt = 0;
  resetValidationFailureCount(); // Reset on successful connection
  useHud.getState().setGateway("connected");
}

export function markDisconnected(): void {
  connected = false;
  useHud.getState().setGateway("disconnected");
}

export function scheduleReconnect(): void {
  if (intentionalDisconnect || reconnectTimer) return;
  const delay = computeBackoffMs(reconnectAttempt);
  reconnectAttempt = Math.min(reconnectAttempt + 1, 10);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect();
  }, delay);
}

export function clearReconnect(): void {
  if (!reconnectTimer) return;
  clearTimeout(reconnectTimer);
  reconnectTimer = null;
}

export function computeBackoffMs(attempt: number, rand: () => number = Math.random): number {
  const safeAttempt = Math.max(0, Math.min(10, attempt));
  const base = Math.min(30_000, 1_000 * 2 ** safeAttempt);
  const jitter = Math.floor(rand() * 400) - 200;
  return Math.max(500, base + jitter);
}

export function isConnected(): boolean {
  return connected;
}

export function _resetCadisActionsForTest(): void {
  disconnect();
  intentionalDisconnect = false;
  reconnectAttempt = 0;
  _setLastEventId(null);
  _resetProcessedEventIds();
}

export function _emitCadisSubscriptionFrameForTest(frame: CadisFrame): void {
  handleSubscriptionFrame(frame);
}
