/**
 * Curated voice catalog. Edge TTS exposes hundreds; we surface the ones most
 * useful for an Indonesian/English bilingual user.
 */
export type VoiceOption = {
  id: string;
  label: string;
  locale: "id-ID" | "en-US" | "en-GB" | "ms-MY";
  gender: "Female" | "Male";
};

export const VOICES: VoiceOption[] = [
  // Indonesian
  { id: "id-ID-ArdiNeural",   label: "Ardi (Indonesian, Male)",   locale: "id-ID", gender: "Male"   },
  { id: "id-ID-GadisNeural",  label: "Gadis (Indonesian, Female)", locale: "id-ID", gender: "Female" },
  // Malay (close fallback)
  { id: "ms-MY-OsmanNeural",  label: "Osman (Malay, Male)",        locale: "ms-MY", gender: "Male"   },
  { id: "ms-MY-YasminNeural", label: "Yasmin (Malay, Female)",     locale: "ms-MY", gender: "Female" },
  // English (US)
  { id: "en-US-AvaNeural",    label: "Ava (US, Female)",           locale: "en-US", gender: "Female" },
  { id: "en-US-AndrewNeural", label: "Andrew (US, Male)",          locale: "en-US", gender: "Male"   },
  { id: "en-US-EmmaNeural",   label: "Emma (US, Female)",          locale: "en-US", gender: "Female" },
  { id: "en-US-BrianNeural",  label: "Brian (US, Male)",           locale: "en-US", gender: "Male"   },
  // English (GB)
  { id: "en-GB-SoniaNeural",  label: "Sonia (GB, Female)",         locale: "en-GB", gender: "Female" },
  { id: "en-GB-RyanNeural",   label: "Ryan (GB, Male)",            locale: "en-GB", gender: "Male"   },
];

export type VoicePrefs = {
  voiceId: string;
  /** -100 .. +100 (% adjustment) */
  rate: number;
  /** -50 .. +50 (Hz adjustment) */
  pitch: number;
  /** -100 .. +100 (% adjustment) */
  volume: number;
  /** Auto-speak CADIS chat replies. */
  autoSpeak: boolean;
  /** When true, attempt Edge TTS (cloud) before falling back to local synth. */
  useCloudTts: boolean;
};

export const DEFAULT_VOICE_PREFS: VoicePrefs = {
  voiceId: "id-ID-GadisNeural",
  rate: 0,
  pitch: 0,
  volume: 0,
  autoSpeak: true,
  /** Cloud Edge TTS is the default — local webkit2gtk often lacks speechSynthesis. */
  useCloudTts: true,
};

export function fmtPercent(n: number): string {
  const sign = n >= 0 ? "+" : "";
  return `${sign}${n}%`;
}

export function fmtHz(n: number): string {
  const sign = n >= 0 ? "+" : "";
  return `${sign}${n}Hz`;
}
