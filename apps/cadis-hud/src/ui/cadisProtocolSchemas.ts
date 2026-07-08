/**
 * Runtime validation schemas for CADIS daemon protocol responses.
 * 
 * Security Note: These schemas protect against malicious or corrupted daemon
 * responses that could cause XSS, DoS, or state corruption in the HUD.
 * 
 * All daemon responses MUST be validated before being processed by handlers.
 */

import { z } from "zod";

// ============================================================================
// Base Schemas
// ============================================================================

const MAX_STRING_LENGTH = 10_000;
const MAX_ARRAY_LENGTH = 1_000;

/** Safe string with length limit to prevent DoS */
const safeString = z.string().max(MAX_STRING_LENGTH);

/** Safe array with length limit to prevent DoS */
function safeArray<T extends z.ZodTypeAny>(schema: T) {
  return z.array(schema).max(MAX_ARRAY_LENGTH);
}

/** Agent/Session/Worker ID validation (alphanumeric + dash/underscore) */
const idString = z.string().regex(/^[a-zA-Z0-9_-]+$/).max(64);

/** ISO timestamp or epoch milliseconds */
const timestamp = z.union([z.string(), z.number()]);

// ============================================================================
// Daemon Status & Models
// ============================================================================

export const DaemonStatusSchema = z.object({
  model_provider: safeString.optional(),
  uptime_seconds: z.number().nonnegative().optional(),
  version: safeString.optional(),
});

export const ModelSchema = z.object({
  id: safeString.optional(),
  name: safeString.optional(),
  display_name: safeString.optional(),
  provider: safeString.optional(),
  model: safeString.optional(),
  provider_id: safeString.optional(),
  model_id: safeString.optional(),
}).refine(
  (data) => {
    // At least one identifying field must be present
    return data.id || data.name || data.display_name || 
           (data.provider && data.model) || 
           (data.provider_id && data.model_id) ||
           data.model || data.model_id;
  },
  { message: "Model must have at least one identifying field (id, name, provider/model, etc.)" }
);

export const ModelsListSchema = z.object({
  models: safeArray(ModelSchema).optional(),
  default_model: safeString.optional(),
});

// ============================================================================
// Preferences
// ============================================================================

export const PreferencesSchema = z.object({
  preferences: z
    .object({
      hud: z
        .object({
          theme: z.enum(["light", "dark", "auto", "amber", "arc", "phosphor", "violet", "alert", "ice"]).optional(),
          avatar_style: safeString.optional(),
          background_opacity: z.number().min(0).max(1).optional(),
        })
        .optional(),
      voice: z
        .object({
          enabled: z.boolean().optional(),
          provider: z.enum(["edge", "elevenlabs", "openai", "system"]).optional(),
          voice_id: safeString.optional(),
          rate: z.number().min(-50).max(50).optional(),
          pitch: z.number().min(-50).max(50).optional(),
          volume: z.number().min(-50).max(50).optional(),
          auto_speak: z.boolean().optional(),
        })
        .optional(),
      chat: z
        .object({
          thinking: z.boolean().optional(),
          fast: z.boolean().optional(),
        })
        .optional(),
    })
    .optional(),
});

// ============================================================================
// Session & Messages
// ============================================================================

export const SessionStartedSchema = z.object({
  session_id: idString.optional(),
  title: safeString.optional(),
  agent_id: idString.optional(),
});

export const MessageDeltaSchema = z.object({
  delta: safeString,
  agent_id: idString.optional(),
  agent_name: safeString.optional(),
});

export const MessageCompletedSchema = z.object({
  content: safeString.optional(),
  text: safeString.optional(),
  agent_id: idString.optional(),
  agent_name: safeString.optional(),
});

// ============================================================================
// Voice
// ============================================================================

export const VoiceStatusSchema = z.object({
  enabled: z.boolean().optional(),
  state: z.enum(["idle", "speaking", "error", "disabled", "degraded"]).optional(),
  status: z.enum(["idle", "speaking", "error", "disabled"]).optional(),
  provider: safeString.optional(),
  voice_id: safeString.optional(),
  stt_language: safeString.optional(),
  max_spoken_chars: z.number().optional(),
  bridge: safeString.optional(),
  last_preflight: z.object({
    surface: safeString.optional(),
    status: safeString.optional(),
    summary: safeString.optional(),
    checked_at: safeString.optional(),
  }).optional(),
});

export const VoiceCheckSchema = z.object({
  name: safeString,
  status: z.enum(["ok", "warn", "error"]),
  message: safeString,
});

export const VoiceDoctorSchema = z.object({
  status: VoiceStatusSchema.optional(),
  checks: safeArray(VoiceCheckSchema).optional(),
});

// ============================================================================
// Agents
// ============================================================================

export const AgentSpawnedSchema = z.object({
  agent_id: idString.optional(),
  id: idString.optional(),
  display_name: safeString.optional(),
  name: safeString.optional(),
  role: safeString.optional(),
  model: safeString.optional(),
  parent_agent_id: idString.optional(),
  status: z.enum([
    "idle", "working", "waiting", "error",
    // Daemon variants that normalizeAgentStatus handles
    "spawning", "starting", "started", "running",
    "waiting_approval", "waitingapproval",
    "completed", "failed", "timed_out", "cancelled", "canceled"
  ]).optional(),
  specialist: safeString.optional(),
});

export const AgentRenamedSchema = z.object({
  agent_id: idString,
  display_name: safeString,
});

export const AgentModelChangedSchema = z.object({
  agent_id: idString,
  model: safeString,
});

export const AgentSpecialistChangedSchema = z.object({
  agent_id: idString,
  specialist: safeString.optional(),
  specialist_id: idString.optional(),
  specialist_label: safeString.optional(),
  persona: safeString.optional(),
});

export const AgentStatusChangedSchema = z.object({
  agent_id: idString,
  status: z.enum([
    "idle", "working", "waiting", "error",
    // Daemon variants that normalizeAgentStatus handles
    "spawning", "starting", "started", "running",
    "waiting_approval", "waitingapproval",
    "completed", "failed", "timed_out", "cancelled", "canceled"
  ]).optional(),
  task: safeString.optional(),
});

export const AgentListSchema = z.object({
  agents: safeArray(AgentSpawnedSchema).optional(),
});

// ============================================================================
// Agent Sessions
// ============================================================================

export const AgentSessionEventSchema = z.object({
  agent_session_id: idString.optional(),
  id: idString.optional(),
  agent_id: idString.optional(),
  session_id: idString.optional(),
  route_id: idString.optional(),
  parent_agent_id: idString.optional(),
  task: safeString.optional(),
  status: z
    .enum([
      "pending",
      "running",
      "completed",
      "failed",
      "cancelled",
      "timeout",
    ])
    .optional(),
  timeout_at: timestamp.optional(),
  budget_steps: z.number().nonnegative().optional(),
  steps_used: z.number().nonnegative().optional(),
  result: safeString.optional(),
  error: safeString.optional(),
  error_code: safeString.optional(),
});

// ============================================================================
// Approvals
// ============================================================================

export const ApprovalRequestedSchema = z.object({
  approval_id: idString.optional(),
  id: idString.optional(),
  risk_class: safeString.optional(),
  rule_id: safeString.optional(),
  summary: safeString.optional(),
  reason: safeString.optional(),
  title: safeString.optional(),
  command: safeString.optional(),
  cmd: safeString.optional(),
  workspace: safeString.optional(),
  cwd: safeString.optional(),
  agent_id: idString.optional(),
});

export const ApprovalResolvedSchema = z.object({
  approval_id: idString.optional(),
  id: idString.optional(),
});

// ============================================================================
// Orchestrator
// ============================================================================

export const OrchestratorRouteSchema = z.object({
  id: idString.optional(),
  target_agent_id: idString.optional(),
  target: idString.optional(),
  target_agent_name: safeString.optional(),
  source: safeString.optional(),
  reason: safeString.optional(),
});

// ============================================================================
// Workers
// ============================================================================

export const WorkerEventSchema = z.object({
  worker_id: idString.optional(),
  id: idString.optional(),
  agent_id: idString.optional(),
  parent_agent_id: idString.optional(),
  cli: safeString.optional(),
  cwd: safeString.optional(),
  status: z.enum(["running", "completed", "failed", "cancelled"]).optional(),
  summary: safeString.optional(),
  delta: safeString.optional(),
  text: safeString.optional(),
  worktree: z.union([
    safeString,
    z.object({
      worktree_root: safeString.optional(),
      worktree_path: safeString.optional(),
      branch_name: safeString.optional(),
      state: safeString.optional(),
      cleanup_policy: safeString.optional(),
    }),
  ]).optional(),
  artifacts: z.union([
    safeString,
    z.object({
      root: safeString.optional(),
      patch: safeString.optional(),
      test_report: safeString.optional(),
      test_report_status: safeString.optional(),
      summary: safeString.optional(),
      changed_files: safeString.optional(),
      memory_candidates: safeString.optional(),
    }),
  ]).optional(),
  started_at: z.number().optional(),
  updated_at: z.number().optional(),
});

// ============================================================================
// Request Rejected
// ============================================================================

export const RequestRejectedSchema = z.object({
  message: safeString.optional(),
  code: safeString.optional(),
});

// ============================================================================
// Patches & Tests
// ============================================================================

export const PatchCreatedSchema = z.object({
  patch_id: idString.optional(),
  summary: safeString.optional(),
});

export const TestResultSchema = z.object({
  status: safeString.optional(),
  summary: safeString.optional(),
});

// ============================================================================
// Validation Helper
// ============================================================================

/**
 * Validates a payload against a schema, returning parsed data or throwing.
 * 
 * @throws {z.ZodError} if validation fails
 */
export function validate<T>(schema: z.ZodType<T>, payload: unknown): T {
  return schema.parse(payload);
}

/**
 * Validates a payload against a schema, returning parsed data or undefined.
 * Logs validation errors for debugging and reports to error tracking.
 */
export function validateSafe<T>(
  schema: z.ZodType<T>,
  payload: unknown,
  context: string,
  onError?: (error: z.ZodError) => void,
): T | undefined {
  const result = schema.safeParse(payload);
  if (result.success) {
    return result.data;
  }
  
  // Track validation failure globally
  const shouldDisconnect = trackValidationFailure();
  
  // Log detailed error for debugging
  console.error(`[CADIS Protocol Validation] ${context}:`, result.error.format());
  
  // Log sanitized error message (without potentially sensitive payload data)
  const errorSummary = `Protocol validation failed for ${context}: ${result.error.errors
    .map((e) => `${e.path.join(".")}: ${e.message}`)
    .slice(0, 3)
    .join(", ")}`;
  console.warn(`[CADIS Security] ${errorSummary}`);
  
  if (shouldDisconnect) {
    console.error(
      `[CADIS Security] Validation failure threshold exceeded. ` +
      `Total failures: ${getValidationFailureCount()}`
    );
  }
  
  // Call optional error handler (for user notifications)
  onError?.(result.error);
  
  return undefined;
}

/**
 * Creates a user-friendly error message from validation errors.
 * Sanitizes technical details while preserving actionable information.
 */
export function formatValidationError(context: string, error: z.ZodError): string {
  const errorCount = error.errors.length;
  const firstError = error.errors[0];
  
  if (errorCount === 1 && firstError) {
    const path = firstError.path.join(".");
    return `Invalid daemon response (${context}${path ? `.${path}` : ""}): ${firstError.message}`;
  }
  
  return `Invalid daemon response (${context}): ${errorCount} validation errors`;
}

/**
 * Handles protocol validation failure by logging and optionally notifying user.
 * 
 * @param context - Protocol message type (e.g., "daemon.status")
 * @param error - Zod validation error
 * @param notifyUser - Whether to show user-visible notification (default: false for non-critical)
 */
export function handleValidationFailure(
  context: string,
  error: z.ZodError,
  notifyUser = false,
): void {
  const message = formatValidationError(context, error);
  
  if (notifyUser) {
    // This will be imported and used by handlers that need user notification
    // For now, just log - integrator can wire up to toast/notification system
    console.error(`[CADIS] ${message}`);
  }
}

/**
 * Global counter for validation failures (for monitoring/alerting).
 */
let validationFailureCount = 0;
const MAX_VALIDATION_FAILURES_BEFORE_DISCONNECT = 50;

/**
 * Increments validation failure counter and checks if threshold exceeded.
 * Returns true if connection should be terminated due to too many errors.
 */
export function trackValidationFailure(): boolean {
  validationFailureCount++;
  
  if (validationFailureCount >= MAX_VALIDATION_FAILURES_BEFORE_DISCONNECT) {
    console.error(
      `[CADIS Security] Too many protocol validation failures (${validationFailureCount}). ` +
      "Possible daemon compromise or version mismatch. Consider reconnecting."
    );
    return true; // Caller should disconnect
  }
  
  return false;
}

/**
 * Resets validation failure counter (call on successful reconnect).
 */
export function resetValidationFailureCount(): void {
  validationFailureCount = 0;
}

/**
 * Gets current validation failure count (for diagnostics).
 */
export function getValidationFailureCount(): number {
  return validationFailureCount;
}

// ============================================================================
// Type Exports
// ============================================================================

export type DaemonStatus = z.infer<typeof DaemonStatusSchema>;
export type Model = z.infer<typeof ModelSchema>;
export type ModelsList = z.infer<typeof ModelsListSchema>;
export type Preferences = z.infer<typeof PreferencesSchema>;
export type SessionStarted = z.infer<typeof SessionStartedSchema>;
export type MessageDelta = z.infer<typeof MessageDeltaSchema>;
export type MessageCompleted = z.infer<typeof MessageCompletedSchema>;
export type VoiceStatus = z.infer<typeof VoiceStatusSchema>;
export type VoiceCheck = z.infer<typeof VoiceCheckSchema>;
export type VoiceDoctor = z.infer<typeof VoiceDoctorSchema>;
export type AgentSpawned = z.infer<typeof AgentSpawnedSchema>;
export type AgentRenamed = z.infer<typeof AgentRenamedSchema>;
export type AgentModelChanged = z.infer<typeof AgentModelChangedSchema>;
export type AgentSpecialistChanged = z.infer<typeof AgentSpecialistChangedSchema>;
export type AgentStatusChanged = z.infer<typeof AgentStatusChangedSchema>;
export type AgentList = z.infer<typeof AgentListSchema>;
export type AgentSessionEvent = z.infer<typeof AgentSessionEventSchema>;
export type ApprovalRequested = z.infer<typeof ApprovalRequestedSchema>;
export type ApprovalResolved = z.infer<typeof ApprovalResolvedSchema>;
export type OrchestratorRoute = z.infer<typeof OrchestratorRouteSchema>;
export type WorkerEvent = z.infer<typeof WorkerEventSchema>;
export type RequestRejected = z.infer<typeof RequestRejectedSchema>;
export type PatchCreated = z.infer<typeof PatchCreatedSchema>;
export type TestResult = z.infer<typeof TestResultSchema>;
