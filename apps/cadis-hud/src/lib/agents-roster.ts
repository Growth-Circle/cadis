/**
 * 12-agent roster. Until CADIS publishes live status, this provides the seed
 * data so the orbital widgets render with realistic-looking content.
 */
export type AgentSpec = {
  id: string;
  name: string;
  role: string;
  icon: string;
  hue: number;
  tasks: { verb: string; target: string; detail: string }[];
};

export const AGENT_ROSTER: AgentSpec[] = [
  {
    id: "main", name: "CADIS", role: "Orchestrator", icon: "◈", hue: 210,
    tasks: [
      { verb: "Ready",   target: "orchestrator",      detail: "Waiting for session" },
      { verb: "Ready",   target: "agent cluster",     detail: "All systems nominal" },
      { verb: "Ready",   target: "session",           detail: "CADIS ready" },
      { verb: "Ready",   target: "models",            detail: "Applying preference map" },
      { verb: "Ready",   target: "inbox",             detail: "Listening for task" },
    ],
  },
  {
    id: "codex", name: "Codex", role: "Coding", icon: "◇", hue: 200,
    tasks: [
      { verb: "Editing",   target: "auth/session.ts",   detail: "Refactoring token refresh logic" },
      { verb: "Writing",   target: "api/webhook.py",    detail: "POST /events handler" },
      { verb: "Running",   target: "pnpm test",          detail: "142 passed · 3 failed" },
      { verb: "Reviewing", target: "PR #482",           detail: "Suggested 6 changes" },
      { verb: "Debugging", target: "useReducer state",  detail: "Stale closure in callback" },
    ],
  },
  {
    id: "atlas", name: "Atlas", role: "Research", icon: "◈", hue: 38,
    tasks: [
      { verb: "Browsing",    target: "arxiv.org",         detail: "Paper 2404.17521 · RAG survey" },
      { verb: "Cross-ref",   target: "14 sources",        detail: "Consolidating findings" },
      { verb: "Reading",     target: "postgres.org/docs", detail: "JSONB performance" },
      { verb: "Summarizing", target: "32 pages",          detail: "→ 800 words" },
    ],
  },
  {
    id: "forge", name: "Forge", role: "Automation", icon: "◆", hue: 140,
    tasks: [
      { verb: "Syncing",     target: "~/Project",     detail: "rsync → backup-02" },
      { verb: "Renaming",    target: "328 files",      detail: "IMG_* → 2026-04-*" },
      { verb: "Converting",  target: "/videos/*.mov",  detail: "→ mp4 · 12/47" },
      { verb: "Cleaning",    target: "node_modules",   detail: "Freed 2.3 GB" },
    ],
  },
  {
    id: "sentry", name: "Sentry", role: "System", icon: "◉", hue: 10,
    tasks: [
      { verb: "Watching",    target: "cpu · mem · net", detail: "All nominal" },
      { verb: "Alert",       target: "docker/postgres", detail: "RAM at 87%" },
      { verb: "Logging",     target: "/var/log/syslog", detail: "2,341 events/min" },
      { verb: "Tracking",    target: "GPU temp",        detail: "64°C · stable" },
    ],
  },
  {
    id: "bash", name: "Bash", role: "Shell", icon: "▸", hue: 280,
    tasks: [
      { verb: "Running",     target: "docker compose up", detail: "4 services starting" },
      { verb: "Executing",   target: "deploy.sh",          detail: "staging environment" },
      { verb: "Building",    target: "docker image",       detail: "Layer 7/12" },
      { verb: "Connected",   target: "prod-01.internal",   detail: "ssh · root" },
    ],
  },
  {
    id: "mneme", name: "Mneme", role: "Memory", icon: "◊", hue: 320,
    tasks: [
      { verb: "Indexing",    target: "2,481 notes",        detail: "Building embeddings" },
      { verb: "Recalling",   target: '"kubernetes setup"', detail: "14 matches" },
      { verb: "Writing",     target: "daily/2026-04-25",   detail: "New note" },
      { verb: "Linking",     target: "proj-cadis",         detail: "→ 7 backlinks" },
    ],
  },
  {
    id: "chronos", name: "Chronos", role: "Schedule", icon: "◐", hue: 220,
    tasks: [
      { verb: "Next up",     target: "Standup · 10:30",    detail: "in 22 min" },
      { verb: "Blocking",    target: "14:00 – 16:00",      detail: "Deep work" },
      { verb: "Reminding",   target: "Submit invoice",     detail: "Due tomorrow" },
      { verb: "Syncing",     target: "gcal · fastmail",    detail: "12 events merged" },
    ],
  },
  {
    id: "muse", name: "Muse", role: "Creative", icon: "✦", hue: 340,
    tasks: [
      { verb: "Drafting",    target: 'Blog: "HUDs"',       detail: "1,240 / ~2,000 words" },
      { verb: "Sketching",   target: "Logo options",       detail: "3 directions" },
      { verb: "Revising",    target: "Landing copy",       detail: "Tone: crisper" },
      { verb: "Naming",      target: "new project",        detail: "12 candidates" },
    ],
  },
  {
    id: "relay", name: "Relay", role: "Network", icon: "◎", hue: 170,
    tasks: [
      { verb: "Listening",   target: ":3000 :5432 :6379",  detail: "3 services" },
      { verb: "Proxying",    target: "tailscale",          detail: "4 peers online" },
      { verb: "Checking",    target: "dns · tls",          detail: "All green" },
      { verb: "Tunneling",   target: "localhost:3000",     detail: "→ *.ngrok.io" },
    ],
  },
  {
    id: "prism", name: "Prism", role: "Data", icon: "◩", hue: 60,
    tasks: [
      { verb: "Querying",    target: "orders_2026",        detail: "1.4M rows · 340ms" },
      { verb: "Plotting",    target: "revenue by cohort",  detail: "7-day rolling" },
      { verb: "Joining",     target: "users × events",     detail: "4.2M rows" },
      { verb: "Exporting",   target: "dashboard.csv",      detail: "→ ~/reports" },
    ],
  },
  {
    id: "aegis", name: "Aegis", role: "Security", icon: "◬", hue: 0,
    tasks: [
      { verb: "Scanning",    target: "deps tree",          detail: "0 critical · 2 low" },
      { verb: "Rotating",    target: "api keys",           detail: "3/5 done" },
      { verb: "Watching",    target: "auth.log",           detail: "no anomalies" },
      { verb: "Blocking",    target: "47 IPs",             detail: "ssh brute force" },
    ],
  },
  {
    id: "echo", name: "Echo", role: "Voice I/O", icon: "◔", hue: 100,
    tasks: [
      { verb: "Transcribing", target: "mic input",         detail: "whisper-large · local" },
      { verb: "Speaking",     target: "piper-voice",       detail: "2.1s audio" },
      { verb: "Idle",         target: "wake word",         detail: '"Hey CADIS"' },
    ],
  },
];

const STATUSES = [
  "working",
  "working",
  "working",
  "working",
  "idle",
  "idle",
  "waiting",
] as const;
export type AgentStatus = (typeof STATUSES)[number];

export function pickTask(agent: AgentSpec, seed: number) {
  return agent.tasks[seed % agent.tasks.length]!;
}

export function pickStatus(seed: number): AgentStatus {
  return STATUSES[seed % STATUSES.length]!;
}
