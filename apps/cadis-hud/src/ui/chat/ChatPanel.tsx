/**
 * Chat panel - real round-trip to CADIS through the Tauri command adapter.
 *
 *   user types / speaks → sendUserMessage()
 *                       → CADIS routes to the active chat agent
 *                       → assistant message arrives as message delta/completed frames
 *                       → store.pushChat fires
 *                       → if voicePrefs.autoSpeak: edge-tts speaks the text
 *
 * Mic button captures local audio and sends a WAV payload to the Tauri STT command.
 */
import { useState, useRef, useEffect, useMemo } from "react";
import type { KeyboardEvent } from "react";
import { useHud, type AgentLive, type ChatMessage } from "../hudState.js";
import { sendUserMessage } from "../cadisActions.js";
import { speak, stopSpeaking } from "../../lib/voice/tts.js";
import {
  available as sttAvailable,
  startListening,
  type SttDebugSnapshot,
  type SttSession,
} from "../../lib/voice/stt.js";
import { VOICES } from "../../lib/voice/voices.js";

const WAVE_BARS = Array.from({ length: 48 }, (_, i) => i);
const MAX_MENTION_OPTIONS = 8;

export type MentionOption = {
  id: string;
  name: string;
  role: string;
  status: string;
};

function fmtTime(ts: number): string {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function ChatPanel() {
  const messages = useHud((s) => s.chat);
  const push = useHud((s) => s.pushChat);
  const clearChat = useHud((s) => s.clearChat);
  const gateway = useHud((s) => s.gateway);
  const prefs = useHud((s) => s.voicePrefs);
  const voiceState = useHud((s) => s.voiceState);
  const setVoiceState = useHud((s) => s.setVoiceState);
  const openConfig = useHud((s) => s.setConfigOpen);
  const agents = useHud((s) => s.agents);
  const agentModels = useHud((s) => s.agentModels);
  const defaultModel = useHud((s) => s.defaultModel);
  const mainName = useHud((s) => s.agents.find((a) => a.spec.id === "main")?.spec.name ?? "CADIS");
  const [draft, setDraft] = useState("");
  const [mentionIndex, setMentionIndex] = useState(0);
  const [dismissedMentionDraft, setDismissedMentionDraft] = useState<string | null>(null);
  const [listening, setListening] = useState(false);
  const [partial, setPartial] = useState("");
  const [audioLevel, setAudioLevel] = useState(0);
  const [audioSamples, setAudioSamples] = useState<number[]>([]);
  const [micDebugOpen, setMicDebugOpen] = useState(false);
  const [micDebugCapture, setMicDebugCapture] = useState(false);
  const [micDebug, setMicDebug] = useState<SttDebugSnapshot>(() => emptyMicDebug());
  const sttRef = useRef<SttSession | null>(null);
  const voiceSubmittedRef = useRef(false);
  const scroll = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const lastSpokenIdRef = useRef<string | null>(null);
  const mentionQuery = getActiveMentionQuery(draft);
  const mentionOptions = useMemo(
    () => (mentionQuery === null ? [] : buildMentionOptions(agents, mentionQuery)),
    [agents, mentionQuery],
  );
  const showMentionMenu =
    gateway === "connected" &&
    mentionQuery !== null &&
    dismissedMentionDraft !== draft &&
    mentionOptions.length > 0;

  useEffect(() => {
    if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [messages.length]);

  useEffect(() => {
    setMentionIndex(0);
  }, [mentionQuery, mentionOptions.length]);

  useEffect(() => {
    const last = messages[messages.length - 1];
    if (!prefs.autoSpeak && last?.who === "cadis" && last.final !== false) {
      setVoiceState("idle");
    }
  }, [messages, prefs.autoSpeak, setVoiceState]);

  // Auto-speak CADIS final replies immediately; hold back partial streams.
  const lastTextRef = useRef<string>("");
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!prefs.autoSpeak) return;
    const last = messages[messages.length - 1];
    if (!last || last.who !== "cadis") return;
    if (lastSpokenIdRef.current === last.id) return;
    if (last.final === false) {
      lastTextRef.current = last.text;
      if (debounceRef.current) clearTimeout(debounceRef.current);
      return;
    }
    if (last.text === lastTextRef.current && last.final !== true) return;
    lastTextRef.current = last.text;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    const snapshot = last.text;
    const id = last.id;
    const delay = last.final === true ? 80 : 700;
    debounceRef.current = setTimeout(() => {
      if (snapshot !== lastTextRef.current) return; // newer chunk arrived
      lastSpokenIdRef.current = id;
      setVoiceState("speaking");
      speak(snapshot, prefs, {
        onEnd: () => setVoiceState("idle"),
        onError: (err) => {
          setVoiceState("idle");
          const msg = err instanceof Error ? err.message : String(err);
          push({
            id: `m-${Date.now()}-tts`,
            who: "system",
            text: `(tts error: ${msg})`,
            ts: Date.now(),
          });
        },
      }).catch((err) => {
        setVoiceState("idle");
        const msg = err instanceof Error ? err.message : String(err);
        push({
          id: `m-${Date.now()}-tts`,
          who: "system",
          text: `(tts error: ${msg})`,
          ts: Date.now(),
        });
      });
    }, delay);
  }, [messages, prefs, setVoiceState, push]);

  const submitText = (text: string) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    const model = agentModels.main ?? defaultModel ?? undefined;
    const ok = sendUserMessage(trimmed, model);
    if (ok) setVoiceState("thinking");
    push({
      id: `m-${Date.now()}`,
      who: "user",
      text: trimmed,
      ts: Date.now(),
    });
    if (!ok) {
      push({
        id: `m-${Date.now()}-warn`,
        who: "system",
        text: "(CADIS not connected - message could not be delivered)",
        ts: Date.now(),
      });
    }
    setDraft("");
  };

  const submit = () => submitText(draft);

  const applyMention = (option: MentionOption) => {
    const next = `@${option.id} `;
    setDraft(next);
    setDismissedMentionDraft(null);
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(next.length, next.length);
    });
  };

  const handleDraftKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (showMentionMenu) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setMentionIndex((index) => (index + 1) % mentionOptions.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setMentionIndex((index) => (index - 1 + mentionOptions.length) % mentionOptions.length);
        return;
      }
      if ((event.key === "Enter" && !event.shiftKey) || event.key === "Tab") {
        event.preventDefault();
        applyMention(mentionOptions[mentionIndex] ?? mentionOptions[0]!);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissedMentionDraft(draft);
        return;
      }
    }

    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  };

  const sttLang = (() => {
    const v = VOICES.find((x) => x.id === prefs.voiceId);
    return v?.locale ?? "en-US";
  })();

  const stopMicCapture = () => {
    sttRef.current?.stop();
    sttRef.current = null;
    voiceSubmittedRef.current = false;
    setListening(false);
    setMicDebugCapture(false);
    setVoiceState("idle");
    setPartial("");
    setAudioLevel(0);
    setAudioSamples([]);
  };

  const beginMicCapture = async (debugOnly: boolean) => {
    if (listening) {
      stopMicCapture();
      return;
    }
    if (!sttAvailable()) {
      push({
        id: `m-${Date.now()}-stt`,
        who: "system",
        text: "(stt error: microphone capture is not available in this webview)",
        ts: Date.now(),
      });
      return;
    }
    await stopSpeaking();
    setVoiceState("listening");
    setListening(true);
    setMicDebugCapture(debugOnly);
    setAudioLevel(0);
    setAudioSamples([]);
    setMicDebugOpen((open) => open || debugOnly);
    setMicDebug(emptyMicDebug(sttLang));
    voiceSubmittedRef.current = false;
    sttRef.current = startListening(sttLang, {
      onDebug: setMicDebug,
      onLevel: ({ level, samples }) => {
        setAudioLevel(level);
        setAudioSamples(samples);
      },
      onPartial: setPartial,
      onFinal: (t) => {
        setPartial("");
        setAudioLevel(0);
        setAudioSamples([]);
        setListening(false);
        setMicDebugCapture(false);
        sttRef.current = null;
        voiceSubmittedRef.current = true;
        submitText(t);
      },
      onEmpty: ({ message, audioHeard }) => {
        setPartial("");
        setAudioLevel(0);
        setAudioSamples([]);
        setListening(false);
        setMicDebugCapture(false);
        sttRef.current = null;
        voiceSubmittedRef.current = false;
        setVoiceState("idle");
        if (audioHeard || debugOnly) setMicDebugOpen(true);
        if (!debugOnly || !audioHeard) {
          push({
            id: `m-${Date.now()}-stt-empty`,
            who: "system",
            text: debugOnly ? `(mic debug: ${message})` : `(stt status: ${message})`,
            ts: Date.now(),
          });
        }
      },
      onError: (msg) => {
        setAudioLevel(0);
        setAudioSamples([]);
        voiceSubmittedRef.current = false;
        setListening(false);
        setMicDebugCapture(false);
        sttRef.current = null;
        setVoiceState("idle");
        setMicDebugOpen(true);
        push({
          id: `m-${Date.now()}-stterr`,
          who: "system",
          text: `(stt error: ${msg})`,
          ts: Date.now(),
        });
      },
      onEnd: () => {
        setAudioLevel(0);
        setAudioSamples([]);
        setListening(false);
        setMicDebugCapture(false);
        sttRef.current = null;
        if (voiceSubmittedRef.current) {
          voiceSubmittedRef.current = false;
          return;
        }
        setVoiceState("idle");
      },
    }, { debugOnly });
  };

  const toggleMic = () => {
    void beginMicCapture(false);
  };

  const clearHistory = () => {
    clearChat();
    setPartial("");
    setMicDebugOpen(false);
    setMicDebug(emptyMicDebug(sttLang));
  };

  const modelLabel = compactModelLabel(agentModels.main ?? defaultModel ?? "openai/codex");
  const statusLabel = voiceStatusLabel(voiceState, gateway);
  const isAwaitingReply = voiceState === "thinking" && messages[messages.length - 1]?.who === "user";
  const showMicDebug = listening || micDebugOpen || micDebug.stage === "error";
  const canClearHistory = messages.length > 0 || partial || showMicDebug;

  return (
    <section className="chat-panel" aria-label="CADIS chat">
      <header className="chat-panel__head">
        <div className="chat-panel__head-main">
          <span className="chat-panel__brand">▸ VOICE I/O</span>
          <span className="chat-panel__sep">·</span>
          <span className="chat-panel__meta">{mainName} · whisper.cpp · edge-tts</span>
          <span className="chat-panel__sep">·</span>
          <span className="chat-panel__meta">{modelLabel}</span>
        </div>
        <span className={`chat-panel__state chat-panel__state--${voiceState}`}>
          {statusLabel}
        </span>
      </header>
      <div ref={scroll} className="chat-panel__log">
        {messages.length === 0 && gateway === "connected" && (
          <div className="chat-panel__placeholder">{mainName.toLowerCase()} › ready. linked to CADIS.</div>
        )}
        {messages.length === 0 && gateway !== "connected" && (
          <div className="chat-panel__placeholder">
            cadis › {gateway}. waiting for CADIS daemon.
          </div>
        )}
        {messages.map((m) => <ChatLine key={m.id} m={m} />)}
        {listening && !partial && (
          <div className="chat-line chat-line--user chat-line--listening">
            <span className="chat-line__ts">...</span>
            <span className="chat-line__who">{micDebugCapture ? "mic ›" : "you ›"}</span>
            <WaveformLine level={audioLevel} samples={audioSamples} />
          </div>
        )}
        {isAwaitingReply && (
          <div className="chat-line chat-line--cadis chat-line--thinking">
            <span className="chat-line__ts">...</span>
            <span className="chat-line__who">{mainName.toLowerCase()} ›</span>
            <span className="chat-line__text">
              consulting CADIS<span className="chat-line__cursor">▌</span>
            </span>
          </div>
        )}
        {partial && (
          <div className="chat-line chat-line--user chat-line--partial">
            <span className="chat-line__ts">...</span>
            <span className="chat-line__who">you ›</span>
            <span className="chat-line__text">{partial}</span>
          </div>
        )}
        {showMicDebug && (
          <MicDebugPanel
            debug={micDebug}
            level={audioLevel}
            listening={listening}
            debugCapture={micDebugCapture}
          />
        )}
      </div>
      <div className="chat-panel__tools" aria-label="chat tools">
        {messages.length > 0 && (
          <div className="chat-panel__chips" aria-label="quick actions">
            {(["yes", "no", "cancel", "expand"] as const).map((label) => (
              <button
                key={label}
                type="button"
                className="chat-panel__chip"
                onClick={() => submitText(label)}
                disabled={gateway !== "connected"}
              >
                {label}
              </button>
            ))}
          </div>
        )}
        <button
          type="button"
          className="chat-panel__tool"
          onClick={clearHistory}
          disabled={!canClearHistory}
          title="Clear chat history"
        >
          CLEAR CHAT
        </button>
      </div>
      <div className="chat-panel__compose-wrap">
        {showMentionMenu && (
          <div
            id="agent-mention-list"
            className="chat-panel__mentions"
            role="listbox"
            aria-label="agent mentions"
          >
            {mentionOptions.map((option, index) => (
              <button
                key={option.id}
                type="button"
                role="option"
                aria-selected={index === mentionIndex}
                className={`chat-panel__mention${index === mentionIndex ? " chat-panel__mention--active" : ""}`}
                onMouseDown={(event) => event.preventDefault()}
                onMouseEnter={() => setMentionIndex(index)}
                onClick={() => applyMention(option)}
              >
                <span className="chat-panel__mention-handle">@{option.id}</span>
                <span className="chat-panel__mention-name">{option.name}</span>
                <span className="chat-panel__mention-role">{option.role}</span>
                <span className={`chat-panel__mention-status chat-panel__mention-status--${option.status}`}>
                  {option.status}
                </span>
              </button>
            ))}
          </div>
        )}
        <form
          className="chat-panel__compose"
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
          style={gateway !== "connected" ? { opacity: 0.45 } : undefined}
          title={gateway !== "connected" ? "Daemon disconnected" : undefined}
        >
          <button
            type="button"
            className={`chat-panel__icon-btn${listening ? " chat-panel__icon-btn--active" : ""}`}
            onClick={toggleMic}
            title={listening ? "Stop listening" : `Talk to ${mainName}`}
            aria-label="microphone"
          >
            <MicIcon active={listening} />
          </button>
          <button
            type="button"
            className="chat-panel__icon-btn"
            onClick={() => openConfig(true, "voice")}
            title="Voice settings"
            aria-label="voice settings"
          >
            <VoiceSettingsIcon />
          </button>
          <button
            type="button"
            className="chat-panel__icon-btn chat-panel__icon-btn--model"
            onClick={() => openConfig(true, "models")}
            title={`Model settings: ${modelLabel}`}
            aria-label={`model settings: ${modelLabel}`}
          >
            <ModelSettingsIcon />
          </button>
          <textarea
            ref={textareaRef}
            rows={1}
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              setDismissedMentionDraft(null);
            }}
            onKeyDown={handleDraftKeyDown}
            aria-autocomplete="list"
            aria-controls={showMentionMenu ? "agent-mention-list" : undefined}
            aria-expanded={showMentionMenu}
            placeholder={
              gateway === "connected"
                ? "or type a command..."
                : "waiting for CADIS..."
            }
            disabled={gateway !== "connected"}
          />
          <button type="submit" disabled={!draft.trim() || gateway !== "connected"}>
            SEND
          </button>
        </form>
      </div>
    </section>
  );
}

function ChatLine({ m }: { m: ChatMessage }) {
  const whoLabel =
    m.who === "user"
      ? "you ›"
      : m.who === "cadis"
        ? `${(m.agentName ?? "cadis").toLowerCase()} ›`
        : "sys ›";
  return (
    <div className={`chat-line chat-line--${m.who}`}>
      <span className="chat-line__ts">{fmtTime(m.ts)}</span>
      <span className="chat-line__who">{whoLabel}</span>
      <span className="chat-line__text">{m.text}</span>
    </div>
  );
}

function MicDebugPanel({
  debug,
  level,
  listening,
  debugCapture,
}: {
  debug: SttDebugSnapshot;
  level: number;
  listening: boolean;
  debugCapture: boolean;
}) {
  const pct = Math.round(Math.max(0, Math.min(1, level || debug.level)) * 100);
  const selectedDevice = debug.selectedDeviceLabel || debug.trackLabel || "-";
  const streamState = debug.streamActive ? "active" : debug.streamId ? "inactive" : "-";
  return (
    <div className="chat-mic-debug">
      <div className="chat-mic-debug__head">
        <span>{debugCapture ? "mic debug capture" : "mic debug"}</span>
        <span className={`chat-mic-debug__pill chat-mic-debug__pill--${debug.stage}`}>
          {listening ? debug.stage : "standby"}
        </span>
      </div>
      <div className="chat-mic-debug__meter" aria-hidden="true">
        <span style={{ width: `${pct}%` }} />
      </div>
      <div className="chat-mic-debug__grid">
        <DebugCell label="level" value={`${pct}%`} />
        <DebugCell label="rms" value={debug.rms.toFixed(5)} />
        <DebugCell label="peak" value={debug.peak.toFixed(5)} />
        <DebugCell label="voice" value={debug.voiceDetected ? "yes" : "no"} />
        <DebugCell label="permission" value={debug.permissionState || "-"} />
        <DebugCell label="inputs" value={debug.deviceCount ? `${debug.deviceCount}` : "-"} />
        <DebugCell label="elapsed" value={formatMs(debug.elapsedMs)} />
        <DebugCell label="silent" value={formatMs(debug.silentMs)} />
        <DebugCell label="chunks" value={`${debug.chunks}`} />
        <DebugCell label="bytes" value={formatBytes(debug.bytes)} />
        <DebugCell label="capture" value={debug.captureSource || "-"} />
        <DebugCell label="pcm" value={debug.pcmFrames ? `${debug.pcmFrames} frames / ${formatBytes(debug.pcmBytes)}` : "-"} />
        <DebugCell label="stop" value={debug.stopReason || "-"} />
        <DebugCell label="selected" value={selectedDevice} wide />
        <DebugCell label="devices" value={debug.deviceLabels || "-"} wide />
        <DebugCell label="stream" value={streamState} />
        <DebugCell
          label="track state"
          value={`${debug.trackReadyState || "-"} / ${debug.trackMuted ? "muted" : "unmuted"}`}
        />
        <DebugCell
          label="track fmt"
          value={[
            debug.trackChannelCount ? `${debug.trackChannelCount}ch` : "",
            debug.trackSampleRate ? `${debug.trackSampleRate}Hz` : "",
            debug.trackDeviceId ? `id ${debug.trackDeviceId}` : "",
          ].filter(Boolean).join(" ") || "-"}
        />
        <DebugCell label="recorder" value={debug.recorderState || "-"} />
        <DebugCell label="mime" value={debug.mimeType || "-"} wide />
        <DebugCell
          label="audio ctx"
          value={`${debug.audioContextState || "-"} ${debug.sampleRate ? `${debug.sampleRate}Hz` : ""}`}
        />
        <DebugCell label="analyser" value={debug.analyserFrames ? `${debug.analyserFrames} frames` : "-"} />
        <DebugCell label="silence" value={debug.silenceReason || "-"} wide />
        <DebugCell label="lang" value={debug.language || "-"} />
        <DebugCell label="message" value={debug.message || "-"} wide />
        {debug.transcript && <DebugCell label="transcript" value={debug.transcript} wide />}
        {debug.error && <DebugCell label="error" value={debug.error} wide />}
      </div>
    </div>
  );
}

function DebugCell({ label, value, wide = false }: { label: string; value: string; wide?: boolean }) {
  return (
    <span className={`chat-mic-debug__cell${wide ? " chat-mic-debug__cell--wide" : ""}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </span>
  );
}

function WaveformLine({ level, samples }: { level: number; samples: number[] }) {
  const normalized = Math.max(0, Math.min(1, level));
  const gain = Math.pow(normalized, 0.72);
  const values = samples.length === WAVE_BARS.length ? samples : WAVE_BARS.map(() => gain);
  return (
    <span
      className="chat-wave"
      data-signal={normalized > 0.08 ? "active" : "quiet"}
      aria-hidden="true"
    >
      {WAVE_BARS.map((i) => (
        <span
          key={i}
          className="chat-wave__bar"
          style={{
            height: `${3 + Math.max(gain * 0.35, values[i] ?? 0) * 20}px`,
            opacity: `${0.32 + Math.max(gain, values[i] ?? 0) * 0.62}`,
          }}
        />
      ))}
    </span>
  );
}

function emptyMicDebug(language = "auto"): SttDebugSnapshot {
  return {
    stage: "idle",
    message: "idle",
    language,
    elapsedMs: 0,
    level: 0,
    rms: 0,
    peak: 0,
    samples: [],
    voiceDetected: false,
    silentMs: 0,
    chunks: 0,
    bytes: 0,
    pcmFrames: 0,
    pcmBytes: 0,
    captureSource: "",
    permissionState: "",
    deviceCount: 0,
    deviceLabels: "",
    selectedDeviceId: "",
    selectedDeviceLabel: "",
    streamActive: false,
    streamId: "",
    trackLabel: "",
    trackEnabled: false,
    trackMuted: false,
    trackReadyState: "",
    trackDeviceId: "",
    trackGroupId: "",
    trackSampleRate: 0,
    trackChannelCount: 0,
    recorderState: "",
    mimeType: "",
    audioContextState: "",
    sampleRate: 0,
    analyserFftSize: 0,
    analyserFrames: 0,
    silenceReason: "",
    stopReason: "",
    transcript: "",
    error: "",
  };
}

function formatMs(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0ms";
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms)}ms`;
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0B";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}

function MicIcon({ active }: { active: boolean }) {
  return (
    <svg className="chat-panel__icon-svg" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3.5a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0v-5a3 3 0 0 0-3-3Z" />
      <path d="M6.5 10.5v1.1a5.5 5.5 0 0 0 11 0v-1.1" />
      <path d="M12 17.2v3.3" />
      <path d="M8.6 20.5h6.8" />
      {active && <path d="M18.5 7.2c1 1.3 1.5 2.9 1.5 4.8s-.5 3.5-1.5 4.8" />}
    </svg>
  );
}

function VoiceSettingsIcon() {
  return (
    <svg className="chat-panel__icon-svg" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 12h2.8l3.4-4.2v8.4L6.8 12H4Z" />
      <path d="M14 8.5c1.1 1 1.6 2.1 1.6 3.5S15.1 14.5 14 15.5" />
      <path d="M17 5.8c1.9 1.7 2.8 3.8 2.8 6.2s-.9 4.5-2.8 6.2" />
    </svg>
  );
}

function ModelSettingsIcon() {
  return (
    <svg className="chat-panel__icon-svg" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <path d="M9.5 9.5h5v5h-5Z" />
      <path d="M9 3.5v2.5M15 3.5v2.5M9 18v2.5M15 18v2.5M3.5 9h2.5M3.5 15h2.5M18 9h2.5M18 15h2.5" />
    </svg>
  );
}

function voiceStatusLabel(
  state: "idle" | "listening" | "thinking" | "speaking",
  gateway: string,
): string {
  if (gateway !== "connected") return `○ ${gateway.toUpperCase()}`;
  if (state === "listening") return "● LISTENING";
  if (state === "speaking") return "● SPEAKING";
  if (state === "thinking") return "◌ THINKING";
  return "○ IDLE";
}

function compactModelLabel(model: string): string {
  const clean = model.replace(/^openai-codex\//, "").replace(/^openai\//, "");
  return clean.length > 22 ? `${clean.slice(0, 19)}...` : clean;
}

export function getActiveMentionQuery(value: string): string | null {
  const match = value.match(/^@([A-Za-z0-9._-]*)$/);
  return match ? (match[1] ?? "") : null;
}

export function buildMentionOptions(agents: AgentLive[], query: string): MentionOption[] {
  const normalizedQuery = normalizeMentionSearch(query);
  return agents
    .map((agent) => ({
      id: agent.spec.id,
      name: agent.spec.name,
      role: agent.spec.role,
      status: agent.status,
    }))
    .filter((option) => {
      if (!normalizedQuery) return true;
      return [option.id, option.name, option.role].some((value) =>
        normalizeMentionSearch(value).includes(normalizedQuery),
      );
    })
    .sort((left, right) => mentionSortScore(left, normalizedQuery) - mentionSortScore(right, normalizedQuery))
    .slice(0, MAX_MENTION_OPTIONS);
}

function mentionSortScore(option: MentionOption, normalizedQuery: string): number {
  if (!normalizedQuery) return option.id === "main" ? -1 : 0;
  const id = normalizeMentionSearch(option.id);
  const name = normalizeMentionSearch(option.name);
  if (id === normalizedQuery || name === normalizedQuery) return 0;
  if (id.startsWith(normalizedQuery) || name.startsWith(normalizedQuery)) return 1;
  return 2;
}

function normalizeMentionSearch(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "");
}
