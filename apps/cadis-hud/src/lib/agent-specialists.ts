export type AgentSpecialistProfile = {
  id: string;
  label: string;
  persona: string;
};

export const CUSTOM_SPECIALIST_ID = "custom";

export const SPECIALIST_OPTIONS: AgentSpecialistProfile[] = [
  {
    id: "general",
    label: "Generalist",
    persona:
      "Act as a pragmatic generalist. Clarify the task, choose the right approach, and produce actionable results.",
  },
  {
    id: "engineering",
    label: "Engineering",
    persona:
      "Act as a senior software engineer. Focus on implementation quality, tests, maintainability, and concrete code-level tradeoffs.",
  },
  {
    id: "research",
    label: "Research",
    persona:
      "Act as a research analyst. Gather context, compare sources, separate facts from assumptions, and return concise evidence-backed findings.",
  },
  {
    id: "marketing",
    label: "Marketing",
    persona:
      "Act as a senior growth marketer. Translate goals into positioning, audience insights, campaigns, funnels, messaging, and measurable experiments.",
  },
  {
    id: "product",
    label: "Product",
    persona:
      "Act as a product strategist. Frame user problems, define scope, prioritize tradeoffs, and turn ambiguity into shippable product decisions.",
  },
  {
    id: "design",
    label: "Design",
    persona:
      "Act as a UX and visual design specialist. Prioritize usability, hierarchy, interaction details, accessibility, and polished interface behavior.",
  },
  {
    id: "data",
    label: "Data",
    persona:
      "Act as a data specialist. Analyze metrics, schemas, datasets, and queries with attention to correctness and decision usefulness.",
  },
  {
    id: "automation",
    label: "Automation",
    persona:
      "Act as an automation specialist. Design reliable repeatable workflows, scripts, and operational checks with clear failure modes.",
  },
  {
    id: "security",
    label: "Security",
    persona:
      "Act as a security specialist. Identify threats, risky assumptions, policy gaps, and mitigations without bypassing CADIS approvals.",
  },
  {
    id: "operations",
    label: "Operations",
    persona:
      "Act as an operations specialist. Monitor runtime health, diagnose incidents, and recommend low-risk operational actions.",
  },
  {
    id: "finance",
    label: "Finance",
    persona:
      "Act as a finance specialist. Focus on unit economics, budgets, pricing, risk, forecasts, and decision-ready financial summaries.",
  },
  {
    id: "writing",
    label: "Writing",
    persona:
      "Act as an editorial writing specialist. Produce clear, audience-aware copy with strong structure, tone control, and concise revisions.",
  },
];

export const CUSTOM_SPECIALIST_OPTION: AgentSpecialistProfile = {
  id: CUSTOM_SPECIALIST_ID,
  label: "Custom",
  persona: "",
};

const ROLE_DEFAULTS: Record<string, string> = {
  orchestrator: "general",
  coding: "engineering",
  research: "research",
  automation: "automation",
  system: "operations",
  shell: "operations",
  memory: "research",
  schedule: "product",
  creative: "writing",
  network: "operations",
  data: "data",
  security: "security",
  "voice i/o": "design",
};

export function specialistOption(id: string | undefined): AgentSpecialistProfile | undefined {
  return SPECIALIST_OPTIONS.find((option) => option.id === id);
}

export function defaultSpecialistForRole(role: string): AgentSpecialistProfile {
  const id = ROLE_DEFAULTS[role.trim().toLowerCase()] ?? "general";
  return SPECIALIST_OPTIONS.find((option) => option.id === id) ?? SPECIALIST_OPTIONS[0]!;
}

export function normalizeSpecialistProfile(
  profile: Partial<AgentSpecialistProfile> | undefined,
  fallback: AgentSpecialistProfile,
): AgentSpecialistProfile {
  const id = normalizeId(profile?.id) || fallback.id;
  const option = specialistOption(id);
  const label = normalizeLabel(profile?.label) || option?.label || fallback.label;
  const persona = normalizePersona(profile?.persona) || option?.persona || fallback.persona;
  return { id, label, persona };
}

export function buildCustomSpecialist(label: string, persona: string): AgentSpecialistProfile {
  const normalizedLabel = normalizeLabel(label) || "Custom";
  const normalizedPersona =
    normalizePersona(persona) ||
    `Act as a ${normalizedLabel} specialist. Use that expertise to interpret tasks and produce actionable results.`;
  return {
    id: CUSTOM_SPECIALIST_ID,
    label: normalizedLabel,
    persona: normalizedPersona,
  };
}

function normalizeId(value: string | undefined): string {
  return (value ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
}

function normalizeLabel(value: string | undefined): string {
  return (value ?? "").trim().replace(/\s+/g, " ").slice(0, 48);
}

function normalizePersona(value: string | undefined): string {
  return (value ?? "").trim().replace(/\s+/g, " ").slice(0, 1200);
}
