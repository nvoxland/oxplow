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
///   event session.created (a child, `info.parentID`)
///                       -> SubagentStart (the parent's, `agent_id` the child)
///   event session.idle / session.error of a child
///                       -> SubagentStop (carries the child's last message,
///                          and the error when it ended in one)
///   event session.error of a session -> kept for its Stop: an abort
///                          (`MessageAbortedError`) or an error, and the
///                          children still running at the Stop ride it as
///                          `background_tasks`
///   event permission.asked / question.asked
///                       -> Notification (a permission prompt, naming it) /
///                          PreToolUse of an `AskUserQuestion` call (the
///                          first question): the agent waits on the person.
///   event permission.replied / question.replied|rejected
///                       -> PostToolUse  (names no tool, or the Ask call:
///                          the wait is over). Only forwarded: the person
///                          answers in opencode's own UI, never here.
///
/// Recording (`OXPLOW_OPENCODE_RAW`, set by oxplow on the launch only while
/// `OXPLOW_HOOK_DEBUG` is set): every plugin hook input/output and every
/// event is first posted as it came, unfiltered and untranslated, to
/// `<base>/opencode.raw`, which oxplow appends to the hook recording as
/// its own line. Then the translation above runs as usual.
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

  // The raw recording, on only when oxplow asked for it.
  const recording = !!env.OXPLOW_OPENCODE_RAW;
  const raw = (kind, payload) => (recording ? post("opencode.raw", { kind, ...payload }) : null);

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

  // Tool calls timed from their two hooks (opencode's carry no duration),
  // the children a session started that have not stopped, and the last
  // error each session reported (kept for its next Stop).
  const toolStarts = new Map();
  const children = new Map();
  const errors = new Map();

  // A child session ended: its parent's SubagentStop, once.
  async function stopChild(childID, error) {
    const child = children.get(childID);
    if (!child) return;
    children.delete(childID);
    const last = await lastAssistant(childID);
    await post("SubagentStop", {
      session_id: child.parent,
      agent_id: childID,
      agent_type: "task",
      last_assistant_message: textOf(last),
      ...(error ? { error: errorOf(error) } : {}),
    });
  }

  // The text parts of an assistant message, in order.
  const textOf = (message) =>
    (message?.parts || [])
      .filter((p) => p && p.type === "text" && typeof p.text === "string")
      .map((p) => p.text)
      .join("\n");

  // An error as oxplow reads it: its name, and its message when it has one.
  const errorOf = (error) => ({
    name: typeof error?.name === "string" ? error.name : "UnknownError",
    ...(typeof error?.data?.message === "string" ? { message: error.data.message } : {}),
  });

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
      await raw("hook", { name: "chat.message", input, output });
      const sessionID = input.sessionID || output.message?.sessionID;
      if (await isChildSession(sessionID)) return;
      const text = (output.parts || [])
        .filter((p) => p && p.type === "text" && typeof p.text === "string")
        .map((p) => p.text)
        .join("\n");
      errors.delete(sessionID);
      const context = contextOf(
        await post("UserPromptSubmit", {
          session_id: sessionID,
          prompt: text,
          // The turn's id in opencode: the user message it opens.
          message_id: output.message?.id ?? input.messageID,
        }),
      );
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
      await raw("hook", { name: "tool.execute.before", input, output });
      if (input.callID) toolStarts.set(input.callID, Date.now());
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
      await raw("hook", { name: "tool.execute.after", input, output });
      // Its bash tool reports the command's exit code as `metadata.exit`;
      // other tools have none, and send none.
      const exit = output?.metadata?.exit;
      const startedAt = toolStarts.get(input.callID);
      toolStarts.delete(input.callID);
      const res = await post("PostToolUse", {
        ...(await whose(input.sessionID)),
        tool_use_id: input.callID,
        tool_name: toolName(input.tool),
        tool_input: toolInput(input.args),
        ...(startedAt !== undefined ? { duration_ms: Date.now() - startedAt } : {}),
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
      await raw("event", { type: event.type, event });
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
      if (event.type === "session.created") {
        const info = props.info;
        if (info?.id && info.parentID) {
          parentCache.set(info.id, info.parentID);
          children.set(info.id, { parent: info.parentID, description: info.title });
          await post("SubagentStart", {
            session_id: info.parentID,
            agent_id: info.id,
            agent_type: "task",
          });
        }
        return;
      }
      if (event.type === "session.error") {
        const sessionID = props.sessionID;
        if (!sessionID) return;
        if (children.has(sessionID)) {
          await stopChild(sessionID, props.error);
          return;
        }
        if (props.error) errors.set(sessionID, props.error);
        return;
      }
      if (event.type === "session.compacted") {
        await post("PostCompact", await whose(props.sessionID));
        return;
      }
      if (event.type !== "session.idle") return;
      const sessionID = props.sessionID;
      if (await isChildSession(sessionID)) {
        await stopChild(sessionID, null);
        return;
      }
      const last = await lastAssistant(sessionID);
      const error = errors.get(sessionID);
      errors.delete(sessionID);
      // The children still running: the work this turn left behind.
      const running = [...children.entries()]
        .filter(([, child]) => child.parent === sessionID)
        .map(([id, child]) => ({
          id,
          type: "subagent",
          status: "running",
          ...(child.description ? { description: child.description } : {}),
        }));
      // Its text parts, in order; the model, variant and tokens it ran on;
      // how it ended when it did not simply finish (an abort or an error).
      await post("Stop", {
        session_id: sessionID,
        last_assistant_message: textOf(last),
        model: last?.info?.modelID,
        variant: last?.info?.variant,
        tokens: last?.info?.tokens,
        background_tasks: running,
        ...(error ? { error: errorOf(error) } : {}),
      });
    },
  };
};
