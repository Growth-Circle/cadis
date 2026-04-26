import { describe, expect, it } from "vitest";
import { describeSttCaptureSource, selectSttCaptureKind } from "./stt.js";

describe("STT capture selection", () => {
  it("keeps MediaRecorder as the primary path when encoded chunks exist", () => {
    expect(selectSttCaptureKind(2, 8192)).toBe("mediarecorder");
  });

  it("falls back to WebAudio PCM when MediaRecorder produces no chunks", () => {
    expect(selectSttCaptureKind(0, 8192)).toBe("webaudio-pcm");
  });

  it("reports no usable capture source when neither path has samples", () => {
    expect(selectSttCaptureKind(0, 0)).toBeNull();
  });

  it("keeps debug telemetry explicit when both capture paths are active", () => {
    expect(describeSttCaptureSource(2, 8192)).toBe("webaudio-pcm+mediarecorder");
    expect(describeSttCaptureSource(0, 8192)).toBe("webaudio-pcm");
    expect(describeSttCaptureSource(2, 0)).toBe("mediarecorder");
  });
});
