import { invoke } from "@tauri-apps/api/core";

export type SttHandlers = {
  onPartial?: (text: string) => void;
  onFinal: (text: string) => void;
  onError?: (msg: string) => void;
  onEnd?: () => void;
  onDebug?: (state: SttDebugSnapshot) => void;
  onLevel?: (state: { level: number; rms: number }) => void;
  onLoading?: (state: { stage: "model" | "recording" | "transcribing" }) => void;
};

export type SttSession = { stop: () => void };

export type SttDebugSnapshot = {
  stage: "idle" | "requesting" | "recording" | "transcribing" | "done" | "error";
  message: string;
  language: string;
  elapsedMs: number;
  level: number;
  rms: number;
  voiceDetected: boolean;
  silentMs: number;
  chunks: number;
  bytes: number;
  trackLabel: string;
  trackEnabled: boolean;
  trackMuted: boolean;
  trackReadyState: string;
  recorderState: string;
  mimeType: string;
  audioContextState: string;
  sampleRate: number;
  stopReason: string;
  transcript: string;
  error: string;
};

type LocalSttResult = {
  text: string;
  latencyMs: number;
};

const SAMPLE_RATE = 16_000;
const VOICE_RMS = 0.006;
const NO_SIGNAL_RMS = 0.0015;
const SILENCE_TAIL_MS = 1800;
const MIN_RECORDING_MS = 1400;
const NO_SIGNAL_TIMEOUT_MS = 10_000;
const MAX_DURATION_MS = 30_000;

export function available(): boolean {
  return typeof navigator !== "undefined" && Boolean(navigator.mediaDevices?.getUserMedia);
}

export function startListening(lang: string, handlers: SttHandlers): SttSession {
  const sessionStartedAt = Date.now();
  let stopped = false;
  let finalizing = false;
  let mediaStream: MediaStream | null = null;
  let recorder: MediaRecorder | null = null;
  let analyserCtx: AudioContext | null = null;
  let heardVoice = false;
  let noSignalTimedOut = false;
  let stopReason = "";
  let chunkBytes = 0;
  const chunks: Blob[] = [];
  let debug: SttDebugSnapshot = {
    stage: "idle",
    message: "idle",
    language: whisperLanguageFromLocale(lang),
    elapsedMs: 0,
    level: 0,
    rms: 0,
    voiceDetected: false,
    silentMs: 0,
    chunks: 0,
    bytes: 0,
    trackLabel: "",
    trackEnabled: false,
    trackMuted: false,
    trackReadyState: "",
    recorderState: "",
    mimeType: "",
    audioContextState: "",
    sampleRate: 0,
    stopReason: "",
    transcript: "",
    error: "",
  };

  const emitDebug = (patch: Partial<SttDebugSnapshot>) => {
    debug = {
      ...debug,
      elapsedMs: Date.now() - sessionStartedAt,
      chunks: chunks.length,
      bytes: chunkBytes,
      recorderState: recorder?.state ?? debug.recorderState,
      audioContextState: analyserCtx?.state ?? debug.audioContextState,
      ...patch,
    };
    handlers.onDebug?.(debug);
  };

  const cleanup = () => {
    mediaStream?.getTracks().forEach((track) => track.stop());
    mediaStream = null;
    if (analyserCtx && analyserCtx.state !== "closed") {
      analyserCtx.close().catch(() => {});
    }
    analyserCtx = null;
    handlers.onLevel?.({ level: 0, rms: 0 });
    emitDebug({ level: 0, rms: 0 });
  };

  const finalize = async () => {
    if (finalizing) return;
    finalizing = true;
    cleanup();

    if (chunks.length === 0) {
      emitDebug({
        stage: "error",
        message: "recording stopped without audio chunks",
        error: "MediaRecorder produced no audio chunks",
        stopReason,
      });
      handlers.onEnd?.();
      return;
    }
    if (noSignalTimedOut && !heardVoice) {
      emitDebug({
        stage: "error",
        message: "no voice signal detected",
        error: "microphone is connected, but CADIS did not receive a voice signal",
        stopReason,
      });
      handlers.onError?.("microphone is connected, but CADIS did not receive a voice signal.");
      handlers.onEnd?.();
      return;
    }

    emitDebug({ stage: "transcribing", message: "sending audio to whisper.cpp", stopReason });
    handlers.onLoading?.({ stage: "transcribing" });
    try {
      const blob = new Blob(chunks, { type: chunks[0]?.type || "audio/webm" });
      const samples = await blobToFloat32Mono16k(blob);
      const wav = encodeWavPcm16(samples, SAMPLE_RATE);
      const audioBase64 = uint8ToBase64(wav);
      const language = whisperLanguageFromLocale(lang);
      const result = await invoke<LocalSttResult>("local_stt_transcribe", { audioBase64, language });
      const text = result.text.trim();
      emitDebug({
        stage: "done",
        message: text ? "transcription complete" : "whisper returned empty text",
        transcript: text,
      });
      if (text) handlers.onFinal(text);
    } catch (err) {
      const message = friendlySttError(err);
      emitDebug({ stage: "error", message: "transcription failed", error: message });
      handlers.onError?.(message);
    } finally {
      handlers.onEnd?.();
    }
  };

  const stopRecording = (reason = "manual") => {
    if (stopped) return;
    stopped = true;
    stopReason = reason;
    emitDebug({ message: `stopping recording: ${reason}`, stopReason: reason });
    try {
      if (recorder?.state === "recording") {
        recorder.stop();
      } else {
        void finalize();
      }
    } catch {
      void finalize();
    }
  };

  (async () => {
    try {
      emitDebug({ stage: "requesting", message: "requesting microphone stream" });
      handlers.onLoading?.({ stage: "recording" });
      mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          autoGainControl: true,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      if (mediaStream.getAudioTracks().length === 0) {
        throw new Error("no microphone audio track was opened");
      }
      const [track] = mediaStream.getAudioTracks();
      if (track) {
        track.onmute = () => emitDebug({ trackMuted: true, message: "microphone track muted" });
        track.onunmute = () => emitDebug({ trackMuted: false, message: "microphone track unmuted" });
        track.onended = () => emitDebug({ trackReadyState: track.readyState, message: "microphone track ended" });
        emitDebug({
          message: `opened microphone track: ${track.label || "unlabeled input"}`,
          trackLabel: track.label,
          trackEnabled: track.enabled,
          trackMuted: track.muted,
          trackReadyState: track.readyState,
        });
      }

      const ctor = (window as unknown as { MediaRecorder?: typeof MediaRecorder }).MediaRecorder;
      if (!ctor) {
        handlers.onError?.("MediaRecorder unavailable in this webview");
        cleanup();
        handlers.onEnd?.();
        return;
      }

      recorder = new ctor(mediaStream, preferredMediaRecorderOptions(ctor));
      recorder.ondataavailable = (event) => {
        if (event.data && event.data.size > 0) {
          chunks.push(event.data);
          chunkBytes += event.data.size;
          emitDebug({
            stage: "recording",
            message: "audio chunk received",
            mimeType: event.data.type || recorder?.mimeType || "",
          });
        }
      };
      recorder.onstop = () => {
        emitDebug({ message: "MediaRecorder stopped", recorderState: recorder?.state ?? "" });
        void finalize();
      };
      recorder.start(150);
      emitDebug({
        stage: "recording",
        message: "recording",
        recorderState: recorder.state,
        mimeType: recorder.mimeType,
      });

      analyserCtx = new AudioContext();
      if (analyserCtx.state === "suspended") {
        await analyserCtx.resume().catch(() => {});
      }
      const src = analyserCtx.createMediaStreamSource(mediaStream);
      const analyser = analyserCtx.createAnalyser();
      analyser.fftSize = 1024;
      analyser.smoothingTimeConstant = 0.42;
      const zeroGain = analyserCtx.createGain();
      zeroGain.gain.value = 0;
      src.connect(analyser);
      analyser.connect(zeroGain);
      zeroGain.connect(analyserCtx.destination);
      emitDebug({
        audioContextState: analyserCtx.state,
        sampleRate: analyserCtx.sampleRate,
        message: "audio analyser connected",
      });

      const buf = new Float32Array(analyser.fftSize);
      let lastVoice = Date.now();
      let lastLevelEmit = 0;
      const startedAt = Date.now();

      const tick = () => {
        if (stopped) return;

        analyser.getFloatTimeDomainData(buf);
        let sum = 0;
        for (let i = 0; i < buf.length; i += 1) {
          sum += buf[i]! * buf[i]!;
        }

        const rms = Math.sqrt(sum / buf.length);
        const now = Date.now();
        if (now - lastLevelEmit > 42) {
          const level = rmsToLevel(rms);
          handlers.onLevel?.({ level, rms });
          emitDebug({
            stage: "recording",
            message: heardVoice ? "voice signal detected" : "waiting for voice signal",
            level,
            rms,
            voiceDetected: heardVoice,
            silentMs: now - lastVoice,
            trackReadyState: track?.readyState ?? debug.trackReadyState,
            trackMuted: track?.muted ?? debug.trackMuted,
            trackEnabled: track?.enabled ?? debug.trackEnabled,
          });
          lastLevelEmit = now;
        }
        if (rms > VOICE_RMS) {
          heardVoice = true;
          lastVoice = now;
        }

        const silentMs = now - lastVoice;
        const totalMs = now - startedAt;
        if (!heardVoice && rms < NO_SIGNAL_RMS && totalMs > NO_SIGNAL_TIMEOUT_MS) {
          noSignalTimedOut = true;
          stopRecording("no-signal");
          return;
        }
        if (heardVoice && totalMs > MIN_RECORDING_MS && silentMs > SILENCE_TAIL_MS) {
          stopRecording("silence");
          return;
        }
        if (totalMs > MAX_DURATION_MS) {
          stopRecording("max-duration");
          return;
        }

        window.setTimeout(tick, 50);
      };

      window.setTimeout(tick, 50);
    } catch (err) {
      stopped = true;
      cleanup();
      const message = friendlySttError(err);
      emitDebug({ stage: "error", message: "microphone capture failed", error: message });
      handlers.onError?.(message);
      handlers.onEnd?.();
    }
  })();

  return { stop: () => stopRecording("manual") };
}

function friendlySttError(err: unknown): string {
  const name = typeof err === "object" && err !== null && "name" in err
    ? String((err as { name?: unknown }).name)
    : "";
  const message = err instanceof Error ? err.message : String(err);
  const normalized = `${name} ${message}`.toLowerCase();
  if (
    normalized.includes("notallowed") ||
    normalized.includes("not allowed") ||
    normalized.includes("permission") ||
    normalized.includes("denied")
  ) {
    return "microphone permission was blocked. Click the mic again and allow microphone access for CADIS in the system prompt/settings.";
  }
  if (normalized.includes("notfound") || normalized.includes("requested device not found")) {
    return "no microphone was found by the system.";
  }
  return message;
}

function rmsToLevel(rms: number): number {
  if (!Number.isFinite(rms) || rms <= 0) return 0;
  return Math.max(0, Math.min(1, Math.sqrt(rms * 18)));
}

function whisperLanguageFromLocale(locale: string): string {
  const base = locale.trim().toLowerCase().split(/[-_]/)[0];
  return base || "auto";
}

function preferredMediaRecorderOptions(ctor: typeof MediaRecorder): { mimeType: string } | undefined {
  const candidates = [
    "audio/webm;codecs=opus",
    "audio/ogg;codecs=opus",
    "audio/mp4",
    "audio/webm",
  ];
  const mimeType = candidates.find((candidate) => ctor.isTypeSupported(candidate));
  return mimeType ? { mimeType } : undefined;
}

async function blobToFloat32Mono16k(blob: Blob): Promise<Float32Array> {
  const arrayBuf = await blob.arrayBuffer();
  const ctx = new AudioContext();
  try {
    const decoded = await ctx.decodeAudioData(arrayBuf.slice(0));
    const channels = decoded.numberOfChannels;
    const len = decoded.length;
    const mono = new Float32Array(len);

    for (let channel = 0; channel < channels; channel += 1) {
      const data = decoded.getChannelData(channel);
      for (let i = 0; i < len; i += 1) {
        mono[i]! += data[i]! / channels;
      }
    }

    const ratio = decoded.sampleRate / SAMPLE_RATE;
    if (ratio === 1) return mono;

    const outLen = Math.floor(len / ratio);
    const out = new Float32Array(outLen);
    for (let i = 0; i < outLen; i += 1) {
      const src = i * ratio;
      const lo = Math.floor(src);
      const hi = Math.min(lo + 1, len - 1);
      const frac = src - lo;
      out[i] = mono[lo]! * (1 - frac) + mono[hi]! * frac;
    }
    return out;
  } finally {
    await ctx.close().catch(() => {});
  }
}

function encodeWavPcm16(samples: Float32Array, sampleRate: number): Uint8Array {
  const bytesPerSample = 2;
  const dataSize = samples.length * bytesPerSample;
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true);
  view.setUint16(32, bytesPerSample, true);
  view.setUint16(34, 8 * bytesPerSample, true);
  writeAscii(view, 36, "data");
  view.setUint32(40, dataSize, true);

  let offset = 44;
  for (let i = 0; i < samples.length; i += 1) {
    const clamped = Math.max(-1, Math.min(1, samples[i]!));
    view.setInt16(offset, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
    offset += bytesPerSample;
  }

  return new Uint8Array(buffer);
}

function writeAscii(view: DataView, offset: number, value: string): void {
  for (let i = 0; i < value.length; i += 1) {
    view.setUint8(offset + i, value.charCodeAt(i));
  }
}

function uint8ToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}
