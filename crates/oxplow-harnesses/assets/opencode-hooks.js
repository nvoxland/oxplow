/// oxplow lifecycle bridge for opencode.
///
/// Loaded via the `plugin` array in the OPENCODE_CONFIG_CONTENT env
/// var oxplow sets at spawn. Translates opencode plugin hooks into the
/// Claude-shaped lifecycle payloads the oxplow control plane already
/// parses (`session_id` / `prompt` / `tool_name` / `tool_input` /
/// `tool_use_id` / `tool_response`), and
/// POSTs them to `$OXPLOW_HOOK_BASE_URL/<Event>` with the session's
/// bearer, which is the whole of who they come from.
///
/// Mappings:
///   chat.message        -> UserPromptSubmit
///   tool.execute.before -> PreToolUse   (deny response -> throw, which
///                          blocks the tool call in opencode)
///   tool.execute.after  -> PostToolUse
///   event session.idle  -> Stop         (closes the turn; never refused)
///
/// Everything is best-effort: a hook POST failure must never break the
/// agent, matching oxplow's side-band hook policy.

/** @type {import("@opencode-ai/plugin").Plugin} */
export const OxplowHooks = async ({ client }) => {
  const env = process.env;
  const base = (env.OXPLOW_HOOK_BASE_URL || "").replace(/\/+$/, "");
  const headers = {
    "Content-Type": "application/json",
    Authorization: `Bearer ${env.OXPLOW_HOOK_TOKEN || ""}`,
  };

  async function post(event, payload) {
    if (!base) return null;
    try {
      const res = await fetch(`${base}/${event}`, {
        method: "POST",
        headers,
        body: JSON.stringify(payload),
      });
      return await res.json().catch(() => null);
    } catch {
      return null;
    }
  }

  // opencode tool names are lowercase; the control plane's write guard
  // matches Claude's names (Edit/Write/Bash/...).
  const TOOL_NAMES = {
    bash: "Bash",
    edit: "Edit",
    patch: "Edit",
    write: "Write",
    read: "Read",
    grep: "Grep",
    glob: "Glob",
    list: "List",
    task: "Task",
    todowrite: "TodoWrite",
    todoread: "TodoRead",
    webfetch: "WebFetch",
  };
  const toolName = (tool) => TOOL_NAMES[tool] || tool;

  // The guards key off Claude's snake_case arg names.
  const toolInput = (args) => {
    const input = { ...(args || {}) };
    if (typeof input.filePath === "string" && input.file_path === undefined) {
      input.file_path = input.filePath;
    }
    return input;
  };

  // Subagent sessions have a parentID; their prompts/idles must not
  // drive the thread's turn lifecycle (mirrors Claude's SubagentStop
  // handling). Cached per session id; on lookup failure assume main.
  const childCache = new Map();
  async function isChildSession(sessionID) {
    if (!sessionID) return false;
    if (childCache.has(sessionID)) return childCache.get(sessionID);
    let child = false;
    try {
      const res = await client.session.get({ path: { id: sessionID } });
      const session = res && typeof res === "object" && "data" in res ? res.data : res;
      child = !!(session && session.parentID);
    } catch {
      child = false;
    }
    childCache.set(sessionID, child);
    return child;
  }

  return {
    "chat.message": async (input, output) => {
      const sessionID = input.sessionID || output.message?.sessionID;
      if (await isChildSession(sessionID)) return;
      const text = (output.parts || [])
        .filter((p) => p && p.type === "text" && typeof p.text === "string")
        .map((p) => p.text)
        .join("\n");
      await post("UserPromptSubmit", { session_id: sessionID, prompt: text });
    },

    "tool.execute.before": async (input, output) => {
      const res = await post("PreToolUse", {
        session_id: input.sessionID,
        tool_use_id: input.callID,
        tool_name: toolName(input.tool),
        tool_input: toolInput(output.args),
      });
      const decision = res?.hookSpecificOutput?.permissionDecision;
      if (decision === "deny") {
        throw new Error(
          res.hookSpecificOutput.permissionDecisionReason || "Blocked by oxplow runtime",
        );
      }
    },

    "tool.execute.after": async (input, output) => {
      // Its bash tool reports the command's exit code as `metadata.exit`;
      // other tools have none, and send none.
      const exit = output?.metadata?.exit;
      await post("PostToolUse", {
        session_id: input.sessionID,
        tool_use_id: input.callID,
        tool_name: toolName(input.tool),
        tool_input: toolInput(input.args),
        tool_response: {
          title: output?.title,
          output: output?.output,
          ...(typeof exit === "number" ? { exit_code: exit } : {}),
        },
      });
    },

    event: async ({ event }) => {
      if (!event || event.type !== "session.idle") return;
      const sessionID = event.properties?.sessionID;
      if (await isChildSession(sessionID)) return;
      await post("Stop", { session_id: sessionID });
    },
  };
};
