import { invoke } from "@tauri-apps/api/core";

export type SttHandlers = {
  onPartial?: (text: string) => void;
  onFinal: (text: string) => void;
  onEmpty?: (state: SttEmptyTranscript) => void;
  onError?: (msg: string) => void;
  onEnd?: () => void;
  onDebug?: (state: SttDebugSnapshot) => void;
  onLevel?: (state: SttLevelSnapshot) => void;
  onLoading?: (state: { stage: "model" | "recording" | "transcribing" }) => void;
};

export type SttSession = { stop: () => void };

export type SttOptions = {
  debugOnly?: boolean;
  maxDurationMs?: number;
  noSignalTimeoutMs?: number;
  silenceTailMs?: number;
};

export type SttEmptyTranscript = {
  message: string;
  audioHeard: boolean;
  stopReason: string;
};

export type SttLevelSnapshot = {
  level: number;
  rms: number;
  peak: number;
  samples: number[];
};

export type SttDebugSnapshot = {
  stage: "idle" | "requesting" | "recording" | "transcribing" | "done" | "error";
  message: string;
  language: string;
  elapsedMs: number;
  level: number;
  rms: number;
  peak: number;
  samples: number[];
  voiceDetected: boolean;
  silentMs: number;
  chunks: number;
  bytes: number;
  permissionState: string;
  deviceCount: number;
  deviceLabels: string;
  selectedDeviceId: string;
  selectedDeviceLabel: string;
  streamActive: boolean;
  streamId: string;
  trackLabel: string;
  trackEnabled: boolean;
  trackMuted: boolean;
  trackReadyState: string;
  trackDeviceId: string;
  trackGroupId: string;
  trackSampleRate: number;
  trackChannelCount: number;
  recorderState: string;
  mimeType: string;
  audioContextState: string;
  sampleRate: number;
  analyserFftSize: number;
  analyserFrames: number;
  silenceReason: string;
  stopReason: string;
  transcript: string;
  error: string;
};

type LocalSttResult = {
  text: string;
  latencyMs: number;
};

type AudioInputDevice = {
  kind: string;
  label: string;
  deviceId: string;
};

const SAMPLE_RATE = 16_000;
const VOICE_RMS = 0.006;
const VOICE_PEAK = 0.035;
const NO_SIGNAL_RMS = 0.0015;
const NO_SIGNAL_PEAK = 0.006;
const SILENCE_TAIL_MS = 1800;
const MIN_RECORDING_MS = 1400;
const NO_SIGNAL_TIMEOUT_MS = 10_000;
const MAX_DURATION_MS = 30_000;
const WAVEFORM_BARS = 48;

export function available(): boolean {
  return typeof navigator !== "undefined" && Boolean(navigator.mediaDevices?.getUserMedia);
}

export function startListening(lang: string, handlers: SttHandlers, options: SttOptions = {}): SttSession {
  const sessionStartedAt = Date.now();
  const debugOnly = options.debugOnly ?? false;
  const maxDurationMs = options.maxDurationMs ?? (debugOnly ? 6_000 : MAX_DURATION_MS);
  const noSignalTimeoutMs = options.noSignalTimeoutMs ?? (debugOnly ? 3_000 : NO_SIGNAL_TIMEOUT_MS);
  const silenceTailMs = options.silenceTailMs ?? SILENCE_TAIL_MS;
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
    peak: 0,
    samples: [],
    voiceDetected: false,
    silentMs: 0,
    chunks: 0,
    bytes: 0,
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
    handlers.onLevel?.({ level: 0, rms: 0, peak: 0, samples: [] });
    emitDebug({ level: 0, rms: 0, peak: 0, samples: [], streamActive: false });
  };

  const finalize = async () => {
    if (finalizing) return;
    finalizing = true;
    cleanup();

    if (chunks.length === 0) {
      if (debugOnly) {
        const message = heardVoice
          ? "mic debug complete: input signal detected"
          : "mic debug complete: no voice threshold crossed";
        emitDebug({
          stage: "done",
          message,
          stopReason,
          silenceReason: debug.silenceReason || "debug capture ended",
        });
        handlers.onEmpty?.({ message, audioHeard: heardVoice, stopReason });
        handlers.onEnd?.();
        return;
      }
      emitDebug({
        stage: "error",
        message: "recording stopped without audio chunks",
        error: "MediaRecorder produced no audio chunks",
        stopReason,
      });
      handlers.onEnd?.();
      return;
    }
    if (debugOnly) {
      const message = heardVoice
        ? "mic debug complete: recorder and analyser received input"
        : "mic debug complete: recorder ran, but voice stayed below threshold";
      emitDebug({
        stage: "done",
        message,
        stopReason,
        silenceReason: debug.silenceReason || "debug capture ended",
      });
      handlers.onEmpty?.({ message, audioHeard: heardVoice, stopReason });
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
      if (text) {
        handlers.onFinal(text);
      } else {
        handlers.onEmpty?.({
          message: heardVoice
            ? "audio was heard, but whisper returned no transcript"
            : "whisper returned no transcript",
          audioHeard: heardVoice,
          stopReason,
        });
      }
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
      const permissionState = await queryMicrophonePermission();
      const devicesBeforePermission = await enumerateAudioInputs();
      emitDebug({
        permissionState,
        deviceCount: devicesBeforePermission.length,
        deviceLabels: summarizeAudioInputs(devicesBeforePermission),
        message: `microphone permission: ${permissionState}`,
      });
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
      const devicesAfterPermission = await enumerateAudioInputs();
      if (track) {
        track.onmute = () => emitDebug({ trackMuted: true, message: "microphone track muted" });
        track.onunmute = () => emitDebug({ trackMuted: false, message: "microphone track unmuted" });
        track.onended = () => emitDebug({ trackReadyState: track.readyState, message: "microphone track ended" });
        const settings = track.getSettings?.() ?? {};
        const selectedDeviceId = shortDeviceId(settings.deviceId);
        const selectedDeviceLabel =
          track.label ||
          devicesAfterPermission.find((device) => device.deviceId === settings.deviceId)?.label ||
          "default audio input";
        emitDebug({
          message: `opened microphone track: ${selectedDeviceLabel}`,
          permissionState: await queryMicrophonePermission(),
          deviceCount: devicesAfterPermission.length || devicesBeforePermission.length,
          deviceLabels: summarizeAudioInputs(devicesAfterPermission.length ? devicesAfterPermission : devicesBeforePermission),
          selectedDeviceId,
          selectedDeviceLabel,
          streamActive: mediaStream.active,
          streamId: shortDeviceId(mediaStream.id),
          trackLabel: track.label,
          trackEnabled: track.enabled,
          trackMuted: track.muted,
          trackReadyState: track.readyState,
          trackDeviceId: selectedDeviceId,
          trackGroupId: shortDeviceId(settings.groupId),
          trackSampleRate: settings.sampleRate ?? 0,
          trackChannelCount: settings.channelCount ?? 0,
        });
      }

      analyserCtx = createAudioContext();
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
        analyserFftSize: analyser.fftSize,
        message: "audio analyser connected",
      });

      const ctor = (window as unknown as { MediaRecorder?: typeof MediaRecorder }).MediaRecorder;
      if (!ctor && !debugOnly) {
        emitDebug({
          stage: "error",
          message: "MediaRecorder unavailable in this webview",
          error: "MediaRecorder unavailable in this webview",
          silenceReason: "capture opened, but this WebKit build cannot encode audio for STT",
        });
        handlers.onError?.("MediaRecorder unavailable in this webview");
        cleanup();
        handlers.onEnd?.();
        return;
      }

      if (ctor && !debugOnly) {
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
      }
      emitDebug({
        stage: "recording",
        message: debugOnly ? "mic debug recording" : "recording",
        recorderState: recorder?.state ?? (debugOnly ? "debug-only" : ""),
        mimeType: recorder?.mimeType ?? "",
      });

      const buf = new Float32Array(analyser.fftSize);
      let lastVoice = Date.now();
      let lastLevelEmit = 0;
      let analyserFrames = 0;
      const startedAt = Date.now();

      const tick = () => {
        if (stopped) return;

        analyser.getFloatTimeDomainData(buf);
        let sum = 0;
        for (let i = 0; i < buf.length; i += 1) {
          sum += buf[i]! * buf[i]!;
        }

        const rms = Math.sqrt(sum / buf.length);
        const peak = peakAmplitude(buf);
        const voiceNow = rms > VOICE_RMS || peak > VOICE_PEAK;
        const now = Date.now();
        analyserFrames += 1;
        if (voiceNow) {
          heardVoice = true;
          lastVoice = now;
        }
        const silentMs = now - lastVoice;
        const totalMs = now - startedAt;
        const silenceReason = describeSilence({
          audioContextState: analyserCtx?.state ?? "",
          heardVoice,
          peak,
          rms,
          silentMs,
          trackEnabled: track?.enabled ?? false,
          trackMuted: track?.muted ?? false,
          trackReadyState: track?.readyState ?? "",
        });
        if (now - lastLevelEmit > 42) {
          const level = rmsToLevel(rms);
          const samples = waveformSamples(buf, WAVEFORM_BARS);
          handlers.onLevel?.({ level, rms, peak, samples });
          emitDebug({
            stage: "recording",
            message: voiceNow || heardVoice ? "voice signal detected" : "waiting for voice signal",
            level,
            rms,
            peak,
            samples,
            voiceDetected: heardVoice,
            silentMs,
            streamActive: mediaStream?.active ?? false,
            trackReadyState: track?.readyState ?? debug.trackReadyState,
            trackMuted: track?.muted ?? debug.trackMuted,
            trackEnabled: track?.enabled ?? debug.trackEnabled,
            analyserFrames,
            silenceReason,
          });
          lastLevelEmit = now;
        }

        if (!heardVoice && rms < NO_SIGNAL_RMS && peak < NO_SIGNAL_PEAK && totalMs > noSignalTimeoutMs) {
          noSignalTimedOut = true;
          emitDebug({ silenceReason: "no input signal above noise floor before timeout" });
          stopRecording("no-signal");
          return;
        }
        if (!debugOnly && heardVoice && totalMs > MIN_RECORDING_MS && silentMs > silenceTailMs) {
          emitDebug({ silenceReason: "voice ended after trailing silence" });
          stopRecording("silence");
          return;
        }
        if (totalMs > maxDurationMs) {
          emitDebug({ silenceReason: debugOnly ? "debug capture duration reached" : "maximum recording duration reached" });
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

async function queryMicrophonePermission(): Promise<string> {
  const permissions = navigator.permissions;
  if (!permissions?.query) return "unsupported";
  try {
    const status = await permissions.query({ name: "microphone" as never });
    return status.state;
  } catch {
    return "unsupported";
  }
}

async function enumerateAudioInputs(): Promise<AudioInputDevice[]> {
  try {
    const devices = await navigator.mediaDevices?.enumerateDevices?.();
    return devices
      ?.filter((device) => device.kind === "audioinput")
      .map((device) => ({
        kind: device.kind,
        label: device.label,
        deviceId: device.deviceId,
      })) ?? [];
  } catch {
    return [];
  }
}

function summarizeAudioInputs(devices: AudioInputDevice[]): string {
  if (devices.length === 0) return "";
  return devices
    .slice(0, 5)
    .map((device, index) => device.label || `audio input ${index + 1}`)
    .join(", ");
}

function shortDeviceId(value: string | undefined): string {
  if (!value) return "";
  return value.length > 12 ? `${value.slice(0, 4)}...${value.slice(-4)}` : value;
}

function createAudioContext(): AudioContext {
  const Ctor = window.AudioContext || (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor) throw new Error("AudioContext unavailable in this webview");
  return new Ctor();
}

function peakAmplitude(samples: Float32Array): number {
  let peak = 0;
  for (let i = 0; i < samples.length; i += 1) {
    const value = Math.abs(samples[i]!);
    if (value > peak) peak = value;
  }
  return peak;
}

function waveformSamples(samples: Float32Array, bars: number): number[] {
  const out: number[] = [];
  const stride = Math.max(1, Math.floor(samples.length / bars));
  for (let bar = 0; bar < bars; bar += 1) {
    const start = bar * stride;
    const end = Math.min(samples.length, start + stride);
    let peak = 0;
    for (let i = start; i < end; i += 1) {
      peak = Math.max(peak, Math.abs(samples[i]!));
    }
    out.push(Math.max(0, Math.min(1, peak * 9)));
  }
  return out;
}

function describeSilence(state: {
  audioContextState: string;
  heardVoice: boolean;
  peak: number;
  rms: number;
  silentMs: number;
  trackEnabled: boolean;
  trackMuted: boolean;
  trackReadyState: string;
}): string {
  if (state.trackReadyState && state.trackReadyState !== "live") return `track is ${state.trackReadyState}`;
  if (!state.trackEnabled) return "track is disabled";
  if (state.trackMuted) return "track is muted by WebKit/system";
  if (state.audioContextState && state.audioContextState !== "running") return `audio context is ${state.audioContextState}`;
  if (state.rms < NO_SIGNAL_RMS && state.peak < NO_SIGNAL_PEAK) return "no input signal above noise floor";
  if (!state.heardVoice) return "input is below voice threshold";
  return state.silentMs > 0 ? "trailing silence after voice" : "voice signal present";
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
