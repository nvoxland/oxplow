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
///   chat.message        -> UserPromptSubmit (an answer's `additionalContext`
///                          joins the user's message as a synthetic text
///                          part, which the model reads)
///   tool.execute.before -> PreToolUse   (a child session's are posted as
///                          its parent's, run by subagent `agent_type: task`;
///                          deny response -> throw, which
///                          blocks the tool call in opencode)
///   tool.execute.after  -> PostToolUse  (an answer's `additionalContext` is
///                          appended to the tool's result, which the model
///                          reads)
///   event session.idle  -> Stop         (closes the turn; never refused;
///                          carries the agent's last message, so a reply
///                          ending in a question sets waiting, and its
///                          `model` / `variant` / `tokens`)
///   event session.compacted
///                       -> PostCompact  (the session's context was
///                          summarized)
///   event permission.asked / question.asked
///                       -> Notification (a permission prompt, naming it) /
///                          PreToolUse of an `AskUserQuestion` call (the
///                          first question): the agent waits on the person.
///   event permission.replied / question.replied|rejected
///                       -> PostToolUse  (names no tool, or the Ask call:
///                          the wait is over). Only forwarded: the person
///                          answers in opencode's own UI, never here.
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
  // handling), and their tool hooks are posted as the parent's, run by
  // a subagent. Cached per session id; on lookup failure assume main.
  const parentCache = new Map();
  async function parentOf(sessionID) {
    if (!sessionID) return null;
    if (parentCache.has(sessionID)) return parentCache.get(sessionID);
    let parent = null;
    try {
      const res = await client.session.get({ path: { id: sessionID } });
      const session = res && typeof res === "object" && "data" in res ? res.data : res;
      parent = (session && session.parentID) || null;
    } catch {
      parent = null;
    }
    parentCache.set(sessionID, parent);
    return parent;
  }
  const isChildSession = async (sessionID) => !!(await parentOf(sessionID));

  // Who a tool hook comes from: the session itself, or, for a child
  // session, its parent with the child named as the subagent (what
  // Claude's `agent_id`/`agent_type` say).
  async function whose(sessionID) {
    const parent = await parentOf(sessionID);
    return parent
      ? { session_id: parent, agent_id: sessionID, agent_type: "task" }
      : { session_id: sessionID };
  }

  // What oxplow asks the model to read with an answer, if anything.
  const contextOf = (res) => {
    const text = res?.hookSpecificOutput?.additionalContext;
    return typeof text === "string" && text !== "" ? text : null;
  };

  // A part id opencode accepts: `prt_` and a unique tail.
  let partCounter = 0;
  const partId = () =>
    `prt_${Date.now().toString(16)}${(partCounter++).toString(16).padStart(4, "0")}${Math.random()
      .toString(16)
      .slice(2, 10)}`;

  // The session's last assistant message, or null when the session can't
  // be read.
  async function lastAssistant(sessionID) {
    try {
      const res = await client.session.messages({ path: { id: sessionID } });
      const messages = res && typeof res === "object" && "data" in res ? res.data : res;
      return [...(messages || [])].reverse().find((m) => m?.info?.role === "assistant") ?? null;
    } catch {
      return null;
    }
  }

  return {
    "chat.message": async (input, output) => {
      const sessionID = input.sessionID || output.message?.sessionID;
      if (await isChildSession(sessionID)) return;
      const text = (output.parts || [])
        .filter((p) => p && p.type === "text" && typeof p.text === "string")
        .map((p) => p.text)
        .join("\n");
      const context = contextOf(await post("UserPromptSubmit", { session_id: sessionID, prompt: text }));
      if (context) {
        output.parts.push({
          id: partId(),
          sessionID,
          messageID: output.message?.id ?? input.messageID,
          type: "text",
          text: context,
          synthetic: true,
        });
      }
    },

    "tool.execute.before": async (input, output) => {
      const res = await post("PreToolUse", {
        ...(await whose(input.sessionID)),
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
      const res = await post("PostToolUse", {
        ...(await whose(input.sessionID)),
        tool_use_id: input.callID,
        tool_name: toolName(input.tool),
        tool_input: toolInput(input.args),
        tool_response: {
          title: output?.title,
          output: output?.output,
          ...(typeof exit === "number" ? { exit_code: exit } : {}),
        },
      });
      const context = contextOf(res);
      if (context && output) output.output = `${output.output ?? ""}\n\n${context}`;
    },

    event: async ({ event }) => {
      if (!event) return;
      const props = event.properties || {};
      // The agent waits on the person (or no longer does): reported, never
      // answered. A child session's wait is its parent's.
      if (event.type === "permission.asked") {
        const patterns = Array.isArray(props.patterns) ? props.patterns.join(", ") : "";
        await post("Notification", {
          ...(await whose(props.sessionID)),
          notification_type: "permission_prompt",
          message: patterns ? `${props.permission}: ${patterns}` : `${props.permission || "A permission"}`,
        });
        return;
      }
      if (event.type === "question.asked") {
        await post("PreToolUse", {
          ...(await whose(props.sessionID)),
          // The tool call that asked, when it names one, so the two pair.
          tool_use_id: props.tool?.callID || props.id,
          tool_name: "AskUserQuestion",
          tool_input: { questions: props.questions || [] },
        });
        return;
      }
      if (event.type === "permission.replied") {
        await post("PostToolUse", await whose(props.sessionID));
        return;
      }
      if (event.type === "question.replied" || event.type === "question.rejected") {
        await post("PostToolUse", {
          ...(await whose(props.sessionID)),
          tool_use_id: props.requestID,
          tool_name: "AskUserQuestion",
          tool_input: {},
          tool_response: {},
        });
        return;
      }
      if (event.type === "session.compacted") {
        await post("PostCompact", await whose(props.sessionID));
        return;
      }
      if (event.type !== "session.idle") return;
      const sessionID = props.sessionID;
      if (await isChildSession(sessionID)) return;
      const last = await lastAssistant(sessionID);
      // Its text parts, in order; the model, variant and tokens it ran on.
      await post("Stop", {
        session_id: sessionID,
        last_assistant_message: (last?.parts || [])
          .filter((p) => p && p.type === "text" && typeof p.text === "string")
          .map((p) => p.text)
          .join("\n"),
        model: last?.info?.modelID,
        variant: last?.info?.variant,
        tokens: last?.info?.tokens,
      });
    },
  };
};
