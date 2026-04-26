/**
 * TTS — Edge TTS runs through the Tauri/Node side because
 * `edge-tts-universal@1.4.x` requires WebSocket headers that Linux WebKit
 * cannot set from renderer JavaScript. Playback also stays native-side so it
 * does not depend on WebKit/GStreamer audio plugins.
 */
import { invoke } from "@tauri-apps/api/core";
import { fmtHz, fmtPercent, type VoicePrefs } from "./voices.js";

export type SpeakHandlers = {
  onStart?: () => void;
  onEnd?: () => void;
  onError?: (err: unknown) => void;
};

function edgePayload(text: string, prefs: VoicePrefs): Record<string, string> {
  return {
    text,
    voiceId: prefs.voiceId,
    rate: fmtPercent(prefs.rate),
    pitch: fmtHz(prefs.pitch),
    volume: fmtPercent(prefs.volume),
  };
}

export async function stopSpeaking(): Promise<void> {
  try {
    await invoke("edge_tts_stop");
  } catch {
    /* Tauri command unavailable in tests/browser preview */
  }
  if (typeof window !== "undefined" && window.speechSynthesis) {
    try {
      window.speechSynthesis.cancel();
    } catch {
      /* noop */
    }
  }
}

async function speakEdge(text: string, prefs: VoicePrefs, handlers: SpeakHandlers): Promise<void> {
  handlers.onStart?.();
  try {
    await invoke("edge_tts_speak", edgePayload(text, { ...prefs, useCloudTts: true }));
    handlers.onEnd?.();
  } catch (err) {
    throw stringifyError(err);
  }
}

export async function speak(
  text: string,
  prefs: VoicePrefs,
  handlers: SpeakHandlers = {},
): Promise<void> {
  await stopSpeaking();
  try {
    await speakEdge(text, prefs, handlers);
  } catch (err) {
    const normalized = stringifyError(err);
    handlers.onError?.(normalized);
    throw normalized;
  }
}

function stringifyError(err: unknown): Error {
  if (err instanceof Error) return err;
  if (typeof err === "string") return new Error(err);
  if (typeof err === "object" && err !== null) {
    const ev = err as { error?: { code?: number; message?: string }; type?: string; message?: string };
    if (ev.error?.message) return new Error(ev.error.message);
    if (ev.error?.code != null) return new Error(`MediaError code ${ev.error.code}`);
    if (ev.message) return new Error(ev.message);
    if (ev.type) return new Error(`audio event: ${ev.type}`);
  }
  return new Error(String(err));
}

export function listLocalVoices(): { id: string; label: string; locale: string }[] {
  if (typeof window === "undefined" || !window.speechSynthesis) return [];
  return window.speechSynthesis.getVoices().map((v) => ({
    id: v.voiceURI,
    label: `${v.name} (${v.lang})`,
    locale: v.lang,
  }));
}

export async function testAudio(
  prefs: VoicePrefs,
  handlers: SpeakHandlers = {},
  agentName = "CADIS",
): Promise<"edge-tts-universal"> {
  const name = agentName.trim() || "CADIS";
  const text =
    prefs.voiceId.startsWith("id-") || prefs.voiceId.startsWith("ms-")
      ? `Halo, saya ${name}. Audio test berhasil.`
      : `Hello, I'm ${name}. Audio test successful.`;
  await speakEdge(text, { ...prefs, useCloudTts: true }, handlers);
  return "edge-tts-universal";
}
