export type HookEventName =
  | "SessionStart"
  | "SessionEnd"
  | "UserPromptSubmit"
  | "PreToolUse"
  | "PostToolUse"
  | "Stop"
  | "Notification";

interface BaseEvent {
  t: number;
  sessionId?: string;
}

export type NormalizedEvent =
  | (BaseEvent & { kind: "session-start"; cwd?: string })
  | (BaseEvent & { kind: "session-end"; reason?: string })
  | (BaseEvent & { kind: "user-prompt"; prompt: string })
  | (BaseEvent & {
      kind: "tool-use-start";
      toolName: string;
      target?: string;
      input?: unknown;
    })
  | (BaseEvent & {
      kind: "tool-use-end";
      toolName: string;
      status: "ok" | "error";
    })
  | (BaseEvent & { kind: "stop" })
  | (BaseEvent & { kind: "notification"; message: string })
  | (BaseEvent & { kind: "meta"; hookEventName: string; raw: unknown });

export function normalize(eventName: HookEventName | string, payload: any, receivedAt: number): NormalizedEvent {
  const p = payload ?? {};
  const base: BaseEvent = {
    t: receivedAt,
    sessionId: typeof p.session_id === "string" ? p.session_id : undefined,
  };

  switch (eventName) {
    case "SessionStart":
      return { kind: "session-start", ...base, cwd: str(p.cwd) };

    case "SessionEnd":
      return { kind: "session-end", ...base, reason: str(p.reason) ?? str(p.exit_reason) };

    case "UserPromptSubmit":
      return { kind: "user-prompt", ...base, prompt: str(p.prompt) ?? "" };

    case "PreToolUse": {
      const toolName = str(p.tool_name) ?? "";
      return {
        kind: "tool-use-start",
        ...base,
        toolName,
        input: p.tool_input,
        target: extractTarget(toolName, p.tool_input),
      };
    }

    case "PostToolUse":
      return {
        kind: "tool-use-end",
        ...base,
        toolName: str(p.tool_name) ?? "",
        status: derivePostStatus(p.tool_response),
      };

    case "Stop":
      return { kind: "stop", ...base };

    case "Notification":
      return { kind: "notification", ...base, message: str(p.message) ?? "" };

    default:
      return { kind: "meta", ...base, hookEventName: String(eventName), raw: p };
  }
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function extractTarget(toolName: string, input: unknown): string | undefined {
  if (!input || typeof input !== "object") return undefined;
  const obj = input as Record<string, unknown>;
  if (toolName === "Grep" && typeof obj.pattern === "string") return obj.pattern;
  if (typeof obj.file_path === "string") return obj.file_path;
  if (typeof obj.path === "string") return obj.path;
  if (typeof obj.notebook_path === "string") return obj.notebook_path;
  if (typeof obj.command === "string") return truncate(obj.command, 80);
  if (typeof obj.pattern === "string") return obj.pattern;
  if (typeof obj.url === "string") return obj.url;
  if (typeof obj.query === "string") return obj.query;
  return undefined;
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : s.slice(0, n - 1) + "…";
}

function derivePostStatus(resp: unknown): "ok" | "error" {
  if (!resp || typeof resp !== "object") return "ok";
  const obj = resp as Record<string, unknown>;
  if (obj.error != null && obj.error !== "") return "error";
  if (obj.is_error === true) return "error";
  return "ok";
}
