import { create } from "zustand";
import { AGENT_ROSTER, type AgentSpec, type AgentStatus } from "../lib/agents-roster.js";
import {
  defaultSpecialistForRole,
  normalizeSpecialistProfile,
  type AgentSpecialistProfile,
} from "../lib/agent-specialists.js";
import { DEFAULT_VOICE_PREFS, type VoicePrefs } from "../lib/voice/voices.js";
import { THEMES as THEME_DEFS, type ThemeKey } from "../styles/themes.js";

export type { ThemeKey };

const FALLBACK_MAIN_MODEL = "openai/gpt-5.5";
const DEFAULT_BACKGROUND_OPACITY = 82;

export type AgentLive = {
  spec: AgentSpec;
  status: AgentStatus;
  currentTask: { verb: string; target: string; detail: string };
  specialist: AgentSpecialistProfile;
  uptimeSeconds: number;
  parentAgentId?: string;
};

export type ChatMessage = {
  id: string;
  who: "user" | "cadis" | "system";
  text: string;
  ts: number;
  final?: boolean;
  agentId?: string;
  agentName?: string;
};

export type GatewayState = "disconnected" | "connecting" | "connected";
export type ConfigTab = "voice" | "models" | "appearance" | "window";
export type AgentModelMap = Record<string, string>;
export type AvatarStyle = "orb" | "wulan_arc";

export type ApprovalRecord = {
  id: string;
  ruleId: string;
  reason: string;
  cmd: string;
  cwd: string;
  agentId: string;
  ts: number;
  timeoutMs?: number;
};

export type AgentSessionStatus =
  | "started"
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "timed_out"
  | "budget_exceeded";

export type AgentSessionRecord = {
  id: string;
  sessionId: string;
  routeId: string;
  agentId: string;
  parentAgentId?: string;
  task: string;
  status: AgentSessionStatus;
  timeoutAt?: string;
  budgetSteps: number;
  stepsUsed: number;
  result?: string;
  error?: string;
  updatedAt: number;
};

export type WorkerStatus = "spawning" | "running" | "completed" | "failed" | "cancelled";

export type WorkerWorktreeInfo = {
  state?: string;
  branchName?: string;
  worktreePath?: string;
  cleanupPolicy?: string;
};

export type WorkerArtifactInfo = {
  summary?: string;
  patch?: string;
  testReport?: string;
  testReportStatus?: string;
};

export type WorkerRecord = {
  id: string;
  agentId?: string;
  parentAgentId?: string;
  cli?: string;
  cwd?: string;
  status: WorkerStatus;
  lastText?: string;
  reason?: string;
  summary?: string;
  logLineCount: number;
  logTail: string[];
  worktree?: WorkerWorktreeInfo;
  artifacts?: WorkerArtifactInfo;
  startedAt: number;
  updatedAt: number;
};

export type VoiceDiagnosticCheck = {
  name: string;
  status: "pass" | "warn" | "fail";
  detail: string;
};

export type VoiceDaemonStatus = {
  enabled: boolean;
  state: "disabled" | "ready" | "degraded" | "blocked" | "unknown";
  provider: string;
  voiceId: string;
  sttLanguage: string;
  maxSpokenChars: number;
  bridge: string;
  lastPreflight?: {
    surface: string;
    status: string;
    summary: string;
    checkedAt: string;
  };
};

export type VoiceDoctorReport = {
  summary: string;
  checks: VoiceDiagnosticCheck[];
};

export type ChatPreferences = {
  thinking: boolean;
  fast: boolean;
};

export type HudStore = {
  gateway: GatewayState;
  agents: AgentLive[];
  agentSessions: AgentSessionRecord[];
  chat: ChatMessage[];
  approvals: ApprovalRecord[];
  workers: WorkerRecord[];
  selectedWorkerId: string | null;
  codeWorkPanelOpen: boolean;
  theme: ThemeKey;
  avatarStyle: AvatarStyle;
  voiceState: "idle" | "listening" | "thinking" | "speaking";
  voicePrefs: VoicePrefs;
  voiceStatus: VoiceDaemonStatus | null;
  voiceDoctor: VoiceDoctorReport | null;
  voiceConfigOpen: boolean;
  modelsConfigOpen: boolean;
  configOpen: boolean;
  configTab: ConfigTab;
  agentRenameTarget: string | null;
  backgroundOpacity: number;
  availableModels: string[];
  defaultModel: string | null;
  agentModels: AgentModelMap;
  chatPreferences: ChatPreferences;
  setGateway: (g: GatewayState) => void;
  setTheme: (t: ThemeKey) => void;
  setAvatarStyle: (style: AvatarStyle) => void;
  pushChat: (m: ChatMessage) => void;
  upsertChat: (m: ChatMessage) => void;
  clearChat: () => void;
  setAgentStatus: (id: string, s: AgentStatus) => void;
  setAgentTask: (id: string, task: Partial<AgentLive["currentTask"]>) => void;
  setVoiceState: (s: HudStore["voiceState"]) => void;
  updateVoicePrefs: (patch: Partial<VoicePrefs>) => void;
  setVoiceStatus: (status: VoiceDaemonStatus | null) => void;
  setVoiceDoctor: (report: VoiceDoctorReport | null) => void;
  setVoiceConfigOpen: (open: boolean) => void;
  setModelsConfigOpen: (open: boolean) => void;
  setConfigOpen: (open: boolean, tab?: ConfigTab) => void;
  setConfigTab: (tab: ConfigTab) => void;
  setAgentRenameTarget: (agentId: string | null) => void;
  renameAgent: (agentId: string, name: string) => void;
  setBackgroundOpacity: (value: number) => void;
  setAvailableModels: (m: string[], defaultModel: string | null) => void;
  setAgentModel: (agentId: string, model: string) => void;
  setAgentSpecialist: (agentId: string, specialist: AgentSpecialistProfile) => void;
  setChatPreferences: (patch: Partial<ChatPreferences>) => void;
  pushApproval: (a: ApprovalRecord) => void;
  removeApproval: (id: string) => void;
  upsertAgentSession: (session: AgentSessionRecord) => void;
  selectWorker: (id: string) => void;
  setCodeWorkPanelOpen: (open: boolean) => void;
  upsertWorker: (w: WorkerRecord) => void;
  removeWorker: (id: string) => void;
};

export const THEMES: Record<ThemeKey, { hue: number; label: string }> = {
  amber: { hue: THEME_DEFS.amber.hue, label: THEME_DEFS.amber.label },
  arc: { hue: THEME_DEFS.arc.hue, label: THEME_DEFS.arc.label },
  phosphor: { hue: THEME_DEFS.phosphor.hue, label: THEME_DEFS.phosphor.label },
  violet: { hue: THEME_DEFS.violet.hue, label: THEME_DEFS.violet.label },
  alert: { hue: THEME_DEFS.alert.hue, label: THEME_DEFS.alert.label },
  ice: { hue: THEME_DEFS.ice.hue, label: THEME_DEFS.ice.label },
};

export function normalizeAgentName(name: string, fallback = "CADIS"): string {
  const clean = name.trim().replace(/\s+/g, " ").slice(0, 32);
  return clean || fallback;
}

export function normalizeAvatarStyle(value: string | undefined): AvatarStyle | null {
  if (value === "orb" || value === "wulan_arc") return value;
  return null;
}

function clampBackgroundOpacity(value: number): number {
  return Math.max(15, Math.min(100, Math.round(value)));
}

function seedAgentModels(): AgentModelMap {
  return Object.fromEntries(AGENT_ROSTER.map((agent) => [agent.id, FALLBACK_MAIN_MODEL]));
}

function buildSeedAgent(spec: AgentSpec, agentModels: AgentModelMap): AgentLive {
  const specialist = defaultSpecialistForRole(spec.role);
  return {
    spec,
    status: spec.id === "main" ? "idle" : "waiting",
    currentTask: {
      verb: "ready",
      target: spec.id === "main" ? "CADIS agent" : `${spec.name} agent`,
      detail: agentModels[spec.id] ?? FALLBACK_MAIN_MODEL,
    },
    specialist,
    uptimeSeconds: 0,
  };
}

function withAgentName(agent: AgentLive, name: string): AgentLive {
  const fallback = agent.spec.id === "main" ? "CADIS" : agent.spec.role;
  const nextName = normalizeAgentName(name, fallback);
  return {
    ...agent,
    spec: {
      ...agent.spec,
      name: nextName,
    },
    currentTask:
      agent.spec.id === "main"
        ? { ...agent.currentTask, target: `${nextName} agent` }
        : agent.currentTask,
  };
}

const INITIAL_AGENT_MODELS = seedAgentModels();
const INITIAL_AGENTS = AGENT_ROSTER.map((agent) => buildSeedAgent(agent, INITIAL_AGENT_MODELS));

export const useHud = create<HudStore>((set) => ({
  gateway: "disconnected",
  agents: INITIAL_AGENTS,
  agentSessions: [],
  chat: [],
  approvals: [],
  workers: [],
  selectedWorkerId: null,
  codeWorkPanelOpen: false,
  theme: "arc",
  avatarStyle: "orb",
  voiceState: "idle",
  voicePrefs: DEFAULT_VOICE_PREFS,
  voiceStatus: null,
  voiceDoctor: null,
  voiceConfigOpen: false,
  modelsConfigOpen: false,
  configOpen: false,
  configTab: "voice",
  agentRenameTarget: null,
  backgroundOpacity: DEFAULT_BACKGROUND_OPACITY,
  availableModels: [FALLBACK_MAIN_MODEL],
  defaultModel: FALLBACK_MAIN_MODEL,
  agentModels: INITIAL_AGENT_MODELS,
  chatPreferences: { thinking: false, fast: true },
  setGateway: (gateway) => set({ gateway }),
  setTheme: (theme) => set({ theme }),
  setAvatarStyle: (avatarStyle) => set({ avatarStyle }),
  pushChat: (message) => set((s) => ({ chat: [...s.chat, message] })),
  upsertChat: (message) =>
    set((s) => {
      const idx = s.chat.findIndex((candidate) => candidate.id === message.id);
      if (idx === -1) return { chat: [...s.chat, message] };
      const next = [...s.chat];
      next[idx] = message;
      return { chat: next };
    }),
  clearChat: () => set({ chat: [] }),
  setAgentStatus: (id, status) =>
    set((s) => ({
      agents: s.agents.map((agent) => (agent.spec.id === id ? { ...agent, status } : agent)),
    })),
  setAgentTask: (id, task) =>
    set((s) => ({
      agents: s.agents.map((agent) =>
        agent.spec.id === id
          ? { ...agent, currentTask: { ...agent.currentTask, ...task } }
          : agent,
      ),
    })),
  setVoiceState: (voiceState) => set({ voiceState }),
  updateVoicePrefs: (patch) =>
    set((s) => ({ voicePrefs: { ...s.voicePrefs, ...patch } })),
  setVoiceStatus: (voiceStatus) => set({ voiceStatus }),
  setVoiceDoctor: (voiceDoctor) => set({ voiceDoctor }),
  setVoiceConfigOpen: (voiceConfigOpen) =>
    set({ voiceConfigOpen, configOpen: voiceConfigOpen, configTab: "voice" }),
  setModelsConfigOpen: (modelsConfigOpen) =>
    set({ modelsConfigOpen, configOpen: modelsConfigOpen, configTab: "models" }),
  setConfigOpen: (configOpen, tab) =>
    set((s) => {
      const nextTab = tab ?? s.configTab;
      return {
        configOpen,
        configTab: nextTab,
        voiceConfigOpen: configOpen && nextTab === "voice",
        modelsConfigOpen: configOpen && nextTab === "models",
      };
    }),
  setConfigTab: (configTab) =>
    set((s) => ({
      configTab,
      voiceConfigOpen: s.configOpen && configTab === "voice",
      modelsConfigOpen: s.configOpen && configTab === "models",
    })),
  setAgentRenameTarget: (agentRenameTarget) => set({ agentRenameTarget }),
  renameAgent: (agentId, name) =>
    set((s) => ({
      agents: s.agents.map((agent) =>
        agent.spec.id === agentId ? withAgentName(agent, name) : agent,
      ),
    })),
  setBackgroundOpacity: (value) => set({ backgroundOpacity: clampBackgroundOpacity(value) }),
  setAvailableModels: (availableModels, defaultModel) => set({ availableModels, defaultModel }),
  setAgentModel: (agentId, model) =>
    set((s) => ({
      agentModels: { ...s.agentModels, [agentId]: model },
      agents: s.agents.map((agent) =>
        agent.spec.id === agentId
          ? { ...agent, currentTask: { ...agent.currentTask, detail: model } }
          : agent,
      ),
    })),
  setAgentSpecialist: (agentId, specialist) =>
    set((s) => ({
      agents: s.agents.map((agent) =>
        agent.spec.id === agentId
          ? {
              ...agent,
              specialist: normalizeSpecialistProfile(
                specialist,
                defaultSpecialistForRole(agent.spec.role),
              ),
            }
          : agent,
      ),
    })),
  setChatPreferences: (patch) =>
    set((s) => ({ chatPreferences: { ...s.chatPreferences, ...patch } })),
  pushApproval: (approval) =>
    set((s) => {
      const idx = s.approvals.findIndex((candidate) => candidate.id === approval.id);
      if (idx === -1) return { approvals: [...s.approvals, approval] };
      const next = [...s.approvals];
      next[idx] = approval;
      return { approvals: next };
    }),
  removeApproval: (id) => set((s) => ({ approvals: s.approvals.filter((a) => a.id !== id) })),
  upsertAgentSession: (session) =>
    set((s) => {
      const idx = s.agentSessions.findIndex((candidate) => candidate.id === session.id);
      if (idx === -1) return { agentSessions: [...s.agentSessions, session] };
      const next = [...s.agentSessions];
      const definedSession = Object.fromEntries(
        Object.entries(session).filter(([, value]) => value !== undefined),
      ) as AgentSessionRecord;
      next[idx] = { ...next[idx]!, ...definedSession };
      return { agentSessions: next };
    }),
  selectWorker: (selectedWorkerId) => set({ selectedWorkerId, codeWorkPanelOpen: true }),
  setCodeWorkPanelOpen: (codeWorkPanelOpen) => set({ codeWorkPanelOpen }),
  upsertWorker: (worker) =>
    set((s) => {
      const idx = s.workers.findIndex((candidate) => candidate.id === worker.id);
      const shouldAutoSelect = s.selectedWorkerId === null;
      if (idx === -1) {
        return {
          workers: [...s.workers, worker],
          selectedWorkerId: shouldAutoSelect ? worker.id : s.selectedWorkerId,
          codeWorkPanelOpen: shouldAutoSelect ? true : s.codeWorkPanelOpen,
        };
      }
      const next = [...s.workers];
      const definedWorker = Object.fromEntries(
        Object.entries(worker).filter(([, value]) => value !== undefined),
      ) as WorkerRecord;
      next[idx] = { ...next[idx]!, ...definedWorker };
      return {
        workers: next,
        selectedWorkerId: shouldAutoSelect ? worker.id : s.selectedWorkerId,
        codeWorkPanelOpen: shouldAutoSelect ? true : s.codeWorkPanelOpen,
      };
    }),
  removeWorker: (id) =>
    set((s) => ({
      workers: s.workers.filter((w) => w.id !== id),
      selectedWorkerId: s.selectedWorkerId === id ? null : s.selectedWorkerId,
      codeWorkPanelOpen: s.selectedWorkerId === id ? false : s.codeWorkPanelOpen,
    })),
}));

export const selectApprovals = (s: HudStore): ApprovalRecord[] => s.approvals;
export const selectAgentSessions = (s: HudStore): AgentSessionRecord[] => s.agentSessions;
export const selectWorkers = (s: HudStore): WorkerRecord[] => s.workers;
