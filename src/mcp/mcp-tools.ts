import type { ToolDef } from "./mcp-server.js";
import type { ThreadStore, Thread } from "../persistence/thread-store.js";
import type { Stream, StreamStore } from "../persistence/stream-store.js";
import type {
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkItemStore,
} from "../persistence/work-item-store.js";
import type { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import type { FollowupStore } from "../electron/followup-store.js";

export interface McpToolDeps {
  resolveStream(streamId: string | undefined): Stream;
  /** Thread-id-only lookup. Tools accept `threadId` alone; streamId is derived
   *  from the thread row. Handles the case where the agent's prompt
   *  streamId drifted out of sync with reality (the old `resolveThread`
   *  required both args and threw). */
  resolveThreadById(threadId: string): Thread;
  threadStore: ThreadStore;
  streamStore: StreamStore;
  workItemStore: WorkItemStore;
  effortStore: WorkItemEffortStore;
  /** Notify the runtime that the agent invoked `await_user` this turn.
   *  Sets the awaiting-user flag so the next Stop hook allows-stop and
   *  suppresses every directive (commit, audit, ready-work, filing
   *  enforcement). Cleared by the next UserPromptSubmit. Optional for
   *  tests that don't spin up a runtime. */
  markAwaitingUser?: (threadId: string, question: string) => void;
  /** Notify the runtime that the agent invoked a work-item-mutating tool
   *  this turn (create_work_item, update_work_item, complete_task,
   *  file_epic_with_children, transition_work_items, dispatch_work_item).
   *  Used by the filing-enforcement Stop branch. Optional for tests. */
  markFiledThisTurn?: (threadId: string) => void;
  /** Stricter sibling: only called by `create_work_item` /
   *  `file_epic_with_children` when the new row landed at `ready`.
   *  Drives the Stop-hook "filed but didn't ship" advisory branch. */
  markFiledReadyThisTurn?: (threadId: string) => void;
  /** Fork a thread on the same stream: seed with a note item from
   *  `summary`, optionally move `moveItemIds` across. Returns the new
   *  thread id. Optional for tests that don't wire the runtime. */
  forkThread?: (input: {
    sourceThreadId: string;
    title: string;
    summary: string;
    moveItemIds?: string[];
  }) => { newThreadId: string };
  /** Transient in-memory follow-up store. Backs the `add_followup` /
   *  `remove_followup` / `list_followups` MCP tools. Optional only so
   *  unit tests that don't exercise follow-ups can omit it. */
  followupStore?: FollowupStore;
}

// Strip noisy fields off audit events before handing them to the agent.
// Stored payloads include full before/after rows for `updated`; keeping only
// the changed keys typically cuts a 1.5 kB event to ~100 bytes.
export function slimWorkItemEvent(ev: { event_type: string; payload_json: string; created_at: string; actor_kind: string }) {
  const base = { event_type: ev.event_type, actor_kind: ev.actor_kind, created_at: ev.created_at };
  let payload: unknown = {};
  try { payload = JSON.parse(ev.payload_json); } catch { /* payload stays {} */ }
  if (ev.event_type !== "updated" || typeof payload !== "object" || payload === null) {
    return { ...base, payload };
  }
  const p = payload as { before?: Record<string, unknown>; after?: Record<string, unknown> };
  if (!p.before || !p.after) return { ...base, payload };
  const beforeDiff: Record<string, unknown> = {};
  const afterDiff: Record<string, unknown> = {};
  for (const key of Object.keys(p.after)) {
    if (!Object.is(p.before[key], p.after[key])) {
      beforeDiff[key] = p.before[key];
      afterDiff[key] = p.after[key];
    }
  }
  return { ...base, payload: { before: beforeDiff, after: afterDiff } };
}

export function hasAcceptanceCriteria(raw: string | null | undefined): boolean {
  return typeof raw === "string" && raw.trim().length > 0;
}

/** Standing preamble for `dispatch_work_item`. The essentials of the
 *  subagent-protocol skill are embedded here so the brief is
 *  self-contained and token-predictable, even if the subagent hasn't
 *  auto-loaded the SKILL yet. */
const DISPATCH_PREAMBLE = [
  "You are executing oxplow work item. Follow the subagent-protocol:",
  "1. Mark the item `in_progress` via `mcp__oxplow__update_work_item` if it is not already.",
  "2. Use red/green TDD. Mark the item `done` (via `complete_task`) when acceptance criteria are met.",
  "3. End by calling `mcp__oxplow__complete_task({ threadId, itemId, note: \"<detailed summary>\" })`. The detailed work summary lives in the note.",
  "4. Your final returned text must be ONE line:",
  "   oxplow-result: {\"ok\":true,\"itemId\":\"wi-...\",\"status\":\"done\",\"tscClean\":true,\"testsPass\":\"N/0\",\"filesChanged\":N}",
  "   No prose — the orchestrator parses the header and fetches the note only if it needs detail.",
].join("\n");

const DISPATCH_CONSTRAINTS = [
  "Constraints:",
  "- No new runtime dependencies.",
  "- No DB mocks in tests — use `mkdtempSync`.",
  "- Singular table names on any new tables.",
].join("\n");

export interface DispatchBriefInputs {
  threadId: string;
  item: {
    id: string;
    title: string;
    kind: string;
    priority: string;
    description: string | null;
    acceptance_criteria: string | null;
  };
  children?: Array<{
    id: string;
    title: string;
    kind: string;
    description: string | null;
    acceptance_criteria: string | null;
  }>;
  /** Most-recent-first list of up to N recent notes. */
  recentNotes?: Array<{ author: string; createdAt: string; body: string }>;
  extraContext?: string;
}

/** Pure composition — no DB access — so unit tests and the MCP
 *  handler can both exercise it without spinning up a server. */
export function composeDispatchBrief(input: DispatchBriefInputs): string {
  const parts: string[] = [];
  parts.push(DISPATCH_PREAMBLE);
  parts.push("");
  parts.push(`threadId: ${input.threadId}`);
  parts.push("");
  parts.push(`## Work item ${input.item.id}`);
  parts.push(`- title: ${input.item.title}`);
  parts.push(`- kind: ${input.item.kind}`);
  parts.push(`- priority: ${input.item.priority}`);
  if (input.item.description && input.item.description.trim().length > 0) {
    parts.push("");
    parts.push("### Description");
    parts.push(input.item.description);
  }
  if (input.item.acceptance_criteria && input.item.acceptance_criteria.trim().length > 0) {
    parts.push("");
    parts.push("### Acceptance criteria");
    parts.push(input.item.acceptance_criteria);
  }
  if (input.children && input.children.length > 0) {
    parts.push("");
    parts.push("## Children");
    for (const child of input.children) {
      parts.push("");
      parts.push(`### ${child.id} — ${child.title} (${child.kind})`);
      if (child.description && child.description.trim().length > 0) {
        parts.push(child.description);
      }
      if (child.acceptance_criteria && child.acceptance_criteria.trim().length > 0) {
        parts.push("Acceptance criteria:");
        parts.push(child.acceptance_criteria);
      }
    }
  }
  if (input.recentNotes && input.recentNotes.length > 0) {
    parts.push("");
    parts.push("## Recent notes (most-recent first)");
    for (const note of input.recentNotes) {
      parts.push(`- [${note.author} @ ${note.createdAt}] ${note.body}`);
    }
  }
  if (input.extraContext && input.extraContext.trim().length > 0) {
    parts.push("");
    parts.push("## Additional context");
    parts.push(input.extraContext.trim());
  }
  parts.push("");
  parts.push(DISPATCH_CONSTRAINTS);
  return parts.join("\n");
}

/** Pure composition for `oxplow__delegate_query`. The orchestrator passes the
 *  returned string to `Agent(subagent_type='Explore', prompt=…)`. The prompt
 *  tells the subagent what to investigate and how to report findings (a
 *  single `oxplow__record_query_finding` call against the pre-allocated
 *  `noteId`). Pure so it's unit-testable without an MCP server. */
export function composeDelegateQueryPrompt(input: {
  threadId: string;
  question: string;
  focus: string;
  noteId: string;
}): string {
  const parts: string[] = [];
  parts.push("You are an Explore subagent answering one focused exploration question for the orchestrator.");
  parts.push("");
  parts.push(`threadId: ${input.threadId}`);
  parts.push(`noteId: ${input.noteId}`);
  parts.push("");
  parts.push("## Question");
  parts.push(input.question);
  if (input.focus && input.focus.length > 0) {
    parts.push("");
    parts.push("## Focus");
    parts.push(input.focus);
  }
  parts.push("");
  parts.push("## How to report");
  parts.push(
    "When done, call `mcp__oxplow__record_query_finding({ noteId, body })` ONCE with your complete finding. " +
    "The body should be concise, structured prose — file paths, key function names, and the direct answer to the question. " +
    "Do not make code changes. Do not create work items. Read/Grep/Glob only.",
  );
  return parts.join("\n");
}

// Returns true when the description reads like it's hiding the acceptance
// checklist — specifically the literal phrase "acceptance criteria" AND at
// least one bullet-looking line. The second gate keeps legitimate
// discussion-style descriptions (e.g. "the existing acceptance criteria
// said …") from tripping the guard.
export function descriptionLooksLikeEmbeddedCriteria(description: string | null | undefined): boolean {
  if (typeof description !== "string" || description.length === 0) return false;
  if (!/acceptance criteria/i.test(description)) return false;
  return /^\s*[-*]\s+\S/m.test(description);
}

// Redo-detection: a create_work_item call made shortly after closing
// an item to `done` on the same thread is *probably* a redo the user
// just asked for (common pattern: agent ships task, user pushes back,
// agent reflexively files a new "Fix …" task). Surface the candidate
// item id so the response hint can point the agent at the reopen path
// (update_work_item → in_progress) instead.
const REDO_HINT_WINDOW_MS = 10 * 60 * 1000;

function findRecentDoneItem(
  workItemStore: WorkItemStore,
  threadId: string,
): { id: string; title: string } | null {
  const cutoff = Date.now() - REDO_HINT_WINDOW_MS;
  const items = workItemStore.listItems(threadId);
  let candidate: { id: string; title: string; ts: number } | null = null;
  for (const item of items) {
    if (item.status !== "done") continue;
    if (item.author !== "agent") continue;
    const ts = Date.parse(item.updated_at);
    if (!Number.isFinite(ts) || ts < cutoff) continue;
    if (!candidate || ts > candidate.ts) {
      candidate = { id: item.id, title: item.title, ts };
    }
  }
  return candidate ? { id: candidate.id, title: candidate.title } : null;
}

function withRedoHint<T extends Record<string, unknown>>(
  response: T,
  recent: { id: string; title: string } | null,
): T {
  if (!recent) return response;
  return {
    ...response,
    redoHint:
      `Recently closed to done on this thread: ${recent.id} "${recent.title}". ` +
      `If this create is a fix/redo of that item, cancel it and instead: ` +
      `update_work_item ${recent.id} status=in_progress → redo → complete_task back to done. ` +
      `Only keep this new item if it's a genuinely separate concern.`,
  };
}

/**
 * Closing an epic to done / blocked when non-terminal children
 * (ready / in_progress) remain produces a misleading rollup: the
 * Plan-pane `classifyEpic` rollup prefers child statuses over the
 * literal epic status, so the epic flips back into To Do or Blocked
 * even though the agent meant to ship it. This guard rejects the
 * call early with a structured error naming the stale children. The
 * agent must either pass `cascade: true` (so this helper plus
 * `cascadeChildrenStatus` flip the children in the same call) or
 * close the children explicitly first.
 *
 * Returns `null` when the call is allowed, or a structured `{ error }`
 * payload when it's rejected. Non-epic items always return `null` —
 * the guard only kicks in for `kind === "epic"`.
 */
export function checkEpicCascadeGuard(
  workItemStore: WorkItemStore,
  threadId: string,
  itemId: string,
  cascade: boolean | undefined,
): { error: string } | null {
  if (cascade === true) return null;
  const item = workItemStore.getItem(threadId, itemId);
  if (!item || item.kind !== "epic") return null;
  const stale = workItemStore
    .listItems(threadId)
    .filter((child) => child.parent_id === itemId && (child.status === "ready" || child.status === "in_progress"));
  if (stale.length === 0) return null;
  const childList = stale
    .map((child) => `  • ${child.id} (${child.status}) "${child.title}"`)
    .join("\n");
  return {
    error:
      `Epic "${item.title}" (${itemId}) has ${stale.length} non-terminal child${stale.length === 1 ? "" : "ren"} ` +
      `that must close along with it (otherwise the section rollup pulls the epic back into To Do):\n` +
      `${childList}\n\n` +
      `Either pass \`cascade: true\` to flip every non-terminal child to the same target status in one call, ` +
      `or close the children explicitly first via \`mcp__oxplow__transition_work_items\`.`,
  };
}

/**
 * Companion to `checkEpicCascadeGuard`: when `cascade: true` is
 * passed, flip every non-terminal child of the epic (ready /
 * in_progress) to the target status. Each child transitions through
 * `updateItem` so the same side effects fire (efforts, audit log,
 * events) as a manual transition would.
 */
export function cascadeChildrenStatus(
  workItemStore: WorkItemStore,
  threadId: string,
  epicId: string,
  status: "done" | "blocked",
): void {
  const stale = workItemStore
    .listItems(threadId)
    .filter((child) => child.parent_id === epicId && (child.status === "ready" || child.status === "in_progress"));
  for (const child of stale) {
    workItemStore.updateItem({
      threadId,
      itemId: child.id,
      status,
      actorKind: "agent",
      actorId: "mcp",
    });
  }
}

export function buildWorkItemMcpTools(deps: McpToolDeps): ToolDef[] {
  const { resolveStream, resolveThreadById, threadStore, streamStore, workItemStore, effortStore, markAwaitingUser, markFiledThisTurn, markFiledReadyThisTurn, forkThread, followupStore } = deps;

  // Prefer the thread row's own stream_id over whatever streamId the caller
  // passed (or didn't). Returns { thread, stream } — both guaranteed to agree
  // on stream_id. Throws "unknown thread: …" if the threadId doesn't exist.
  function resolveThreadAndStream(args: { streamId?: string; threadId: string }): { thread: Thread; stream: Stream } {
    const thread = resolveThreadById(args.threadId);
    const stream = resolveStream(thread.stream_id);
    return { thread, stream };
  }

  return [
    {
      name: "oxplow__get_thread_context",
      description: "Return stream and thread context. Use this to confirm the active thread id before calling work-item tools.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Optional thread id to resolve within the stream." },
        },
      },
      handler: (args: { streamId?: string; threadId?: string }) => {
        // When the caller names a threadId, derive stream from the thread row
        // itself — the agent's prompt streamId may have drifted. Without
        // threadId, fall back to the current-stream default.
        const { stream, thread: explicitThread } = args.threadId
          ? (() => {
              const b = resolveThreadById(args.threadId!);
              return { stream: resolveStream(b.stream_id), thread: b };
            })()
          : { stream: resolveStream(args.streamId), thread: null as Thread | null };
        const threadState = threadStore.list(stream.id);
        const thread = explicitThread
          ?? threadState.threads.find((candidate) => candidate.id === threadState.selectedThreadId)
          ?? threadState.threads[0]
          ?? null;
        // Cross-stream snapshot — lets the agent notice that "current
        // stream" may have drifted from where it actually writes. Each
        // entry is the would-be active thread in a peer stream (falling
        // back to the first thread if nothing's active yet).
        const otherActiveThreads = streamStore.list()
          .filter((s) => s.id !== stream.id)
          .map((peer) => {
            const peerState = threadStore.list(peer.id);
            const peerActive = peerState.threads.find((b) => b.id === peerState.activeThreadId)
              ?? peerState.threads[0]
              ?? null;
            return {
              streamId: peer.id,
              streamTitle: peer.title,
              threadId: peerActive?.id ?? null,
              threadTitle: peerActive?.title ?? null,
              activeThreadId: peerState.activeThreadId,
            };
          });
        return {
          streamId: stream.id,
          streamTitle: stream.title,
          threadId: thread?.id ?? null,
          threadTitle: thread?.title ?? null,
          activeThreadId: threadState.activeThreadId,
          selectedThreadId: threadState.selectedThreadId,
          otherActiveThreads,
        };
      },
    },
    {
      name: "oxplow__list_thread_work",
      description: "List all tracked work items for one thread, grouped by waiting/in progress/done. Always pass the threadId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
        },
        required: ["threadId"],
      },
      handler: (args: { streamId?: string; threadId: string }) => {
        resolveThreadAndStream(args);
        return workItemStore.getState(args.threadId);
      },
    },
    {
      name: "oxplow__list_ready_work",
      description: "List actionable work items in one thread that are not blocked by unfinished dependencies. Always pass the threadId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
        },
        required: ["threadId"],
      },
      handler: (args: { streamId?: string; threadId: string }) => {
        resolveThreadAndStream(args);
        return workItemStore.listReady(args.threadId).map((i) => ({
          id: i.id,
          title: i.title,
          kind: i.kind,
          priority: i.priority,
          sort_index: i.sort_index,
          parent_id: i.parent_id,
        }));
      },
    },
    {
      name: "oxplow__read_work_options",
      description: "Return the next dispatch unit for the orchestrator. If the highest-priority ready item is an epic, returns the epic and all its ready descendants as one atomic unit. Otherwise returns all ready non-epic items so you can pick one or a related cluster to dispatch. Always pass the threadId from your session context. By default returns a slim shape (id, title, kind, priority, parent_id, status, sort_index) for scanning — call `get_work_item` per id when composing a dispatch brief, or pass `full=true` for the verbose shape (adds description, acceptance_criteria, and link edges).",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          full: { type: "boolean", description: "Optional. When true, include description, acceptance_criteria, and link edges on every item. Default false returns the slim scanning shape." },
        },
        required: ["threadId"],
      },
      handler: (args: { streamId?: string; threadId: string; full?: boolean }) => {
        resolveThreadAndStream(args);
        const result = workItemStore.readWorkOptions(args.threadId);
        if (result.mode === "empty") return { mode: "empty" };
        const full = args.full === true;
        if (result.mode === "epic") {
          return {
            mode: "epic",
            epic: full
              ? {
                  id: result.epic.id,
                  title: result.epic.title,
                  kind: result.epic.kind,
                  priority: result.epic.priority,
                  parent_id: result.epic.parent_id,
                  status: result.epic.status,
                  sort_index: result.epic.sort_index,
                  description: result.epic.description,
                  acceptance_criteria: result.epic.acceptance_criteria,
                }
              : {
                  id: result.epic.id,
                  title: result.epic.title,
                  kind: result.epic.kind,
                  priority: result.epic.priority,
                  parent_id: result.epic.parent_id,
                  status: result.epic.status,
                  sort_index: result.epic.sort_index,
                },
            children: result.children.map(({ item, outgoing, incoming }) => (
              full
                ? {
                    id: item.id,
                    title: item.title,
                    kind: item.kind,
                    priority: item.priority,
                    parent_id: item.parent_id,
                    status: item.status,
                    sort_index: item.sort_index,
                    description: item.description,
                    acceptance_criteria: item.acceptance_criteria,
                    outgoing: outgoing.map((l) => ({ to_item_id: l.to_item_id, link_type: l.link_type })),
                    incoming: incoming.map((l) => ({ from_item_id: l.from_item_id, link_type: l.link_type })),
                  }
                : {
                    id: item.id,
                    title: item.title,
                    kind: item.kind,
                    priority: item.priority,
                    parent_id: item.parent_id,
                    status: item.status,
                    sort_index: item.sort_index,
                  }
            )),
          };
        }
        return {
          mode: "standalone",
          items: result.items.map(({ item, outgoing, incoming }) => (
            full
              ? {
                  id: item.id,
                  title: item.title,
                  kind: item.kind,
                  priority: item.priority,
                  sort_index: item.sort_index,
                  parent_id: item.parent_id,
                  status: item.status,
                  description: item.description,
                  acceptance_criteria: item.acceptance_criteria,
                  outgoing: outgoing.map((l) => ({ to_item_id: l.to_item_id, link_type: l.link_type })),
                  incoming: incoming.map((l) => ({ from_item_id: l.from_item_id, link_type: l.link_type })),
                }
              : {
                  id: item.id,
                  title: item.title,
                  kind: item.kind,
                  priority: item.priority,
                  sort_index: item.sort_index,
                  parent_id: item.parent_id,
                  status: item.status,
                }
          )),
        };
      },
    },
    {
      name: "oxplow__create_work_item",
      description: "Create a new epic/task/subtask/bug/note within one thread. Always pass the threadId from your session context. acceptanceCriteria, priority, and parentId are top-level JSON fields — do not embed them inside description as XML-style tags. For the 'file and close in one call' shortcut (retroactive splits, record rows where edits already shipped), pass `status: \"done\"` (or `\"blocked\"`) together with `touchedFiles`; the server opens and immediately closes an effort so Local History gets attribution just like a normal close. DO NOT use this to record a fix/redo of an item you just closed to `done` — reopen that item instead (`update_work_item` → `in_progress`), do the new effort, then `complete_task` back to `done`. Filing a new task for a redo fragments the history. A genuinely new concern still warrants a new item.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          parentId: { type: "string", description: "Optional parent epic/task id in the same thread." },
          kind: { type: "string", description: "One of epic, task, subtask, bug, or note. Defaults to \"task\" when omitted." },
          title: { type: "string", description: "Short title for the work item." },
          description: { type: "string", description: "Optional longer description of the approach." },
          acceptanceCriteria: { type: "string", description: "Optional plain-text checklist (one criterion per line) defining observable conditions for 'done'." },
          status: { type: "string", description: "Optional initial status. One of: ready, in_progress, blocked, done, canceled, archived." },
          priority: { type: "string", description: "Optional priority: low, medium, high, or urgent." },
          touchedFiles: {
            type: "array",
            description: "Optional list of repo-relative paths the agent edited for this item. Only meaningful when combined with `status: \"done\"` or `\"blocked\"` (the file-and-close shortcut) — the server synthesizes an in_progress→target transition so Local History can attribute writes. Silently ignored for other statuses.",
            items: { type: "string" },
          },
        },
        required: ["threadId", "title"],
      },
      handler: (args: {
        streamId?: string;
        threadId: string;
        parentId?: string;
        kind?: WorkItemKind;
        title: string;
        description?: string;
        acceptanceCriteria?: string | null;
        status?: WorkItemStatus;
        priority?: WorkItemPriority;
        touchedFiles?: string[];
      }) => {
        resolveThreadAndStream(args);
        if (markFiledThisTurn) markFiledThisTurn(args.threadId);
        // The Stop-hook "filed but didn't ship" advisory branch needs
        // to know whether this filing was a new ready row (backlog
        // capture) or already claimed (in_progress). Only signal the
        // advisory flag for the ready / unspecified case.
        if (markFiledReadyThisTurn && (args.status === undefined || args.status === "ready")) {
          markFiledReadyThisTurn(args.threadId);
        }
        // Default to "task" — by far the most common kind. Forcing the agent
        // to declare it on every call produced a guaranteed first-call
        // failure ("missing required field: kind") for trivial fixes.
        const kind: WorkItemKind = args.kind ?? "task";
        // Silent-failure guard: agents sometimes cram the acceptance
        // checklist into `description` instead of the dedicated top-level
        // `acceptanceCriteria` field. The DB accepts it either way, so the
        // mistake shows up only as a UI gap (the Work panel's acceptance
        // column goes empty). Returning a soft error forces a re-call with
        // the criteria promoted to the proper field.
        if (!hasAcceptanceCriteria(args.acceptanceCriteria) && descriptionLooksLikeEmbeddedCriteria(args.description)) {
          return {
            error: "acceptanceCriteria is a top-level JSON field; don't embed it inside description. Re-call oxplow__create_work_item with the checklist in the acceptanceCriteria field (one criterion per line, plain text).",
          };
        }
        // Redo-detection hint: if a done item authored by the agent was
        // closed on this thread within the last 10 minutes, the new
        // create is *probably* a redo the user just asked for. Emit a
        // soft hint pointing at the reopen path. The create still
        // proceeds — genuinely new concerns should get their own row —
        // but the agent sees a nudge if this is actually a redo.
        const recentDone = findRecentDoneItem(workItemStore, args.threadId);
        // File-and-close shortcut: when the caller asks for a terminal
        // status AND passes `touchedFiles`, file at `ready`, then flip to
        // `in_progress` (opens an effort), then flip to the target
        // (closes with attribution). This is the only path by which an
        // item filed "directly" into done/blocked can carry file
        // attribution — otherwise no effort exists to hang the paths off.
        const closesImmediately =
          (args.status === "done" || args.status === "blocked")
            && Array.isArray(args.touchedFiles) && args.touchedFiles.length > 0;
        if (closesImmediately) {
          const created = workItemStore.createItem({
            threadId: args.threadId,
            parentId: args.parentId,
            kind,
            title: args.title,
            description: args.description,
            acceptanceCriteria: args.acceptanceCriteria,
            status: "ready",
            priority: args.priority,
            createdBy: "agent",
            actorId: "mcp",
            author: "agent",
          });
          workItemStore.updateItem({
            threadId: args.threadId,
            itemId: created.id,
            status: "in_progress",
            actorKind: "agent",
            actorId: "mcp",
          });
          workItemStore.updateItem({
            threadId: args.threadId,
            itemId: created.id,
            status: args.status!,
            touchedFiles: args.touchedFiles,
            actorKind: "agent",
            actorId: "mcp",
          });
          return withRedoHint({ ok: true, id: created.id, sort_index: created.sort_index }, recentDone);
        }
        // Items filed directly at `in_progress` never opened an effort
        // because work-item-store fires `kind:"created"`, not
        // `kind:"updated"`, so the runtime's status-transition subscription
        // (which only reacts to updates) skipped them. That left every
        // agent-filed task with an empty Efforts list until the next
        // complete_task tried to close a non-existent effort. Route
        // status:"in_progress" through ready → in_progress so the
        // subscription sees the transition and opens an effort.
        const wantsInProgress = args.status === "in_progress";
        const item = workItemStore.createItem({
          threadId: args.threadId,
          parentId: args.parentId,
          kind,
          title: args.title,
          description: args.description,
          acceptanceCriteria: args.acceptanceCriteria,
          status: wantsInProgress ? "ready" : args.status,
          priority: args.priority,
          createdBy: "agent",
          actorId: "mcp",
          author: "agent",
        });
        if (wantsInProgress) {
          workItemStore.updateItem({
            threadId: args.threadId,
            itemId: item.id,
            status: "in_progress",
            actorKind: "agent",
            actorId: "mcp",
          });
        }
        // Epics filed without children render as one opaque IN PROGRESS row in
        // the UI and defeat the purpose of the rollup. The oxplow-runtime
        // skill already says "file children in the same turn"; surfacing it on
        // the tool response keeps the rule on the critical path instead of
        // shelved in a skill doc. Non-epic responses stay terse — no field is
        // added there so the happy-path log doesn't grow.
        if (kind === "epic") {
          return withRedoHint({
            ok: true,
            id: item.id,
            sort_index: item.sort_index,
            reminder:
              "Epic filed with 0 children. Per oxplow-runtime, file child tasks now (parentId=this id), before starting execution. An epic without children renders as one opaque IN PROGRESS row in the UI.",
          }, recentDone);
        }
        return withRedoHint({ ok: true, id: item.id, sort_index: item.sort_index }, recentDone);
      },
    },
    {
      name: "oxplow__file_epic_with_children",
      description:
        "Atomically create an epic plus its child work items in one call. Preferred over " +
        "calling create_work_item N+1 times because (a) the epic can't end up in the UI " +
        "without children if a follow-up call fails, and (b) sort_index + audit events fire " +
        "in one transaction. Children default kind=\"task\" if unspecified. Server rejects " +
        "empty children arrays — an epic without children is a bug.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          epic: {
            type: "object",
            description: "Fields for the parent epic row.",
            properties: {
              title: { type: "string" },
              description: { type: "string" },
              acceptance_criteria: { type: "string" },
              priority: { type: "string", description: "One of low, medium, high, urgent." },
            },
            required: ["title"],
          },
          children: {
            type: "array",
            description: "Non-empty list of child items to create under the epic. Each child defaults kind=\"task\".",
            items: {
              type: "object",
              properties: {
                title: { type: "string" },
                description: { type: "string" },
                acceptance_criteria: { type: "string" },
                priority: { type: "string" },
                kind: { type: "string", description: "One of task, subtask, bug, note. Defaults to task." },
              },
              required: ["title"],
            },
          },
        },
        required: ["threadId", "epic", "children"],
      },
      handler: (args: {
        streamId?: string;
        threadId: string;
        epic: {
          title: string;
          description?: string;
          acceptance_criteria?: string | null;
          priority?: WorkItemPriority;
        };
        children: Array<{
          title: string;
          description?: string;
          acceptance_criteria?: string | null;
          priority?: WorkItemPriority;
          kind?: WorkItemKind;
        }>;
      }) => {
        resolveThreadAndStream(args);
        if (markFiledThisTurn) markFiledThisTurn(args.threadId);
        // file_epic_with_children always lands rows at the default
        // (ready) status — the schema doesn't accept a status arg —
        // so this always signals the advisory flag.
        if (markFiledReadyThisTurn) markFiledReadyThisTurn(args.threadId);
        const result = workItemStore.fileEpicWithChildren({
          threadId: args.threadId,
          epic: {
            title: args.epic.title,
            description: args.epic.description,
            acceptanceCriteria: args.epic.acceptance_criteria,
            priority: args.epic.priority,
          },
          children: args.children.map((c) => ({
            title: c.title,
            description: c.description,
            acceptanceCriteria: c.acceptance_criteria,
            priority: c.priority,
            kind: c.kind,
          })),
          createdBy: "agent",
          actorId: "mcp",
        });
        return { ok: true, epicId: result.epicId, childIds: result.childIds };
      },
    },
    {
      name: "oxplow__dispatch_work_item",
      description:
        "Compose a subagent dispatch brief server-side from the work item's own fields, " +
        "so the orchestrator doesn't have to Read its description/acceptance criteria/notes " +
        "into chat context. Returns `{ prompt, itemId }`; pass `prompt` directly to the " +
        "general-purpose Agent tool. When `autoStart !== false` (default true) the item is " +
        "atomically transitioned to `in_progress` if currently `ready`; other " +
        "statuses (including `blocked`) are left alone — un-block explicitly via update_work_item first. " +
        "For epics, the brief includes each child's title + AC.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id." },
          itemId: { type: "string", description: "Required id of the work item to dispatch." },
          extraContext: { type: "string", description: "Optional free-form context to append under `## Additional context`. Use for cross-item coordination or decisions the user made in chat that aren't captured in the item itself." },
          autoStart: { type: "boolean", description: "When true (default) transition the item to in_progress if currently ready. Skipped silently for blocked/terminal/in_progress — use update_work_item to un-block first." },
        },
        required: ["threadId", "itemId"],
      },
      handler: (args: { streamId?: string; threadId: string; itemId: string; extraContext?: string; autoStart?: boolean }) => {
        resolveThreadAndStream(args);
        if (markFiledThisTurn) markFiledThisTurn(args.threadId);
        const item = workItemStore.getItem(args.threadId, args.itemId);
        if (!item) throw new Error(`unknown work item: ${args.itemId}`);

        const autoStart = args.autoStart !== false;
        // Only auto-start from `ready`. Per wi-6285706789c5 a blocked item
        // must be explicitly moved to ready first (un-block is a conscious
        // action; dispatch_work_item shouldn't sneak around the guard).
        if (autoStart && item.status === "ready") {
          workItemStore.updateItem({
            threadId: args.threadId,
            itemId: args.itemId,
            status: "in_progress",
            actorKind: "agent",
            actorId: "mcp",
          });
        }

        // Gather children if this is an epic.
        let children: DispatchBriefInputs["children"] = undefined;
        if (item.kind === "epic") {
          const siblings = workItemStore.listItems(args.threadId)
            .filter((c) => c.parent_id === item.id);
          children = siblings.map((c) => ({
            id: c.id,
            title: c.title,
            kind: c.kind,
            description: c.description,
            acceptance_criteria: c.acceptance_criteria,
          }));
        }

        // Gather last 3 notes (most-recent first). Notes live in the
        // work_item_events table as event_type="note".
        const recentNotes = workItemStore.listEvents(args.threadId, args.itemId)
          .filter((e) => e.event_type === "note")
          .slice(0, 3)
          .map((e) => {
            let body = "";
            try { body = (JSON.parse(e.payload_json) as { note?: string }).note ?? ""; } catch { body = ""; }
            return { author: e.actor_kind, createdAt: e.created_at, body };
          });

        const prompt = composeDispatchBrief({
          threadId: args.threadId,
          item: {
            id: item.id,
            title: item.title,
            kind: item.kind,
            priority: item.priority,
            description: item.description,
            acceptance_criteria: item.acceptance_criteria,
          },
          children,
          recentNotes,
          extraContext: args.extraContext,
        });
        return { prompt, itemId: item.id };
      },
    },
    {
      name: "oxplow__complete_task",
      description:
        "Collapse the final add_work_note + status transition into one call. Default " +
        "`status` is `done`. Pass `status: \"blocked\"` instead when you're signalling " +
        "you can't finish and need user input. Pass `touchedFiles` with the repo-relative " +
        "paths you edited so Local History can attribute writes to this item — without it " +
        "the panel falls back to \"assume all\" for this effort. Rejects items whose current " +
        "status is already terminal (done/canceled/archived) — use update_work_item for those.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id." },
          itemId: { type: "string", description: "Required id of the work item to complete." },
          note: { type: "string", description: "Required summary note — what shipped, what's left, where to look." },
          status: { type: "string", description: "Optional. One of `done` (default) or `blocked`." },
          touchedFiles: {
            type: "array",
            description: "Optional list of repo-relative paths the agent edited during this effort. Attaches to the closing effort for Local History attribution. Skip if >100 files — the fallback (assume-all) is fine for large change sets.",
            items: { type: "string" },
          },
          cascade: { type: "boolean", description: "When closing an epic: also flip every non-terminal child (ready/in_progress) to the same target status in one call. Required when the epic has any non-terminal children — otherwise the call is rejected so the children don't silently linger and the epic rollup doesn't pull the epic back into To Do." },
        },
        required: ["threadId", "itemId", "note"],
      },
      handler: (args: {
        streamId?: string;
        threadId: string;
        itemId: string;
        note: string;
        status?: "done" | "blocked";
        touchedFiles?: string[];
        cascade?: boolean;
      }) => {
        resolveThreadAndStream(args);
        if (markFiledThisTurn) markFiledThisTurn(args.threadId);
        // Epic-cascade guard: closing an epic without `cascade: true`
        // when non-terminal children remain produces a misleading
        // rollup (the epic flips back into To Do because
        // classifyEpic prefers child statuses over the literal epic
        // status). Reject early with a structured error naming the
        // stale children. The agent must either pass cascade=true
        // or close the children explicitly first.
        const epicGuardError = checkEpicCascadeGuard(workItemStore, args.threadId, args.itemId, args.cascade);
        if (epicGuardError) return epicGuardError;
        if (args.cascade) {
          cascadeChildrenStatus(workItemStore, args.threadId, args.itemId, args.status ?? "done");
        }
        const item = workItemStore.completeTask({
          threadId: args.threadId,
          itemId: args.itemId,
          note: args.note,
          status: args.status,
          touchedFiles: args.touchedFiles,
          actorKind: "agent",
          actorId: "mcp",
        });
        // Attach the caller's `note` text to the effort that just
        // closed. Efforts (not the work-item note history) own the
        // per-attempt summary now — one summary per effort, written
        // here on the closing transition. The most-recently-closed
        // effort for this item is the one `completeTask` just closed.
        if (typeof args.note === "string" && args.note.length > 0) {
          const allEfforts = effortStore.listEffortsForWorkItem(args.itemId);
          const closed = allEfforts
            .filter((e) => e.ended_at !== null)
            .sort((a, b) => (a.ended_at! < b.ended_at! ? 1 : a.ended_at! > b.ended_at! ? -1 : 0));
          const justClosed = closed[0];
          if (justClosed) {
            effortStore.setEffortSummary(justClosed.id, args.note);
          }
        }
        return { ok: true, id: item.id, status: item.status };
      },
    },
    {
      name: "oxplow__update_work_item",
      description: "Update title, description, acceptance criteria, status, priority, or parent of an existing work item in one thread. Always pass the threadId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          itemId: { type: "string", description: "Required id of the work item to update." },
          parentId: { type: "string", description: "Optional new parent epic/task id." },
          title: { type: "string", description: "Optional replacement title." },
          description: { type: "string", description: "Optional replacement description." },
          acceptanceCriteria: { type: "string", description: "Optional replacement acceptance criteria (checklist). Pass empty string to clear." },
          status: { type: "string", description: "Optional replacement status. One of: ready, in_progress, blocked, done, canceled, archived. Set to 'done' when acceptance criteria are met. 'archived' hides the item from the default Work view." },
          priority: { type: "string", description: "Optional replacement priority." },
          touchedFiles: {
            type: "array",
            description: "Optional list of files the agent touched during this effort. Pass when transitioning to `done` to support parallel-task attribution. Skip if >50 files — the fallback (assume-all) is fine for large change sets.",
            items: { type: "string" },
          },
          cascade: { type: "boolean", description: "When transitioning an epic to `done`/`blocked`: also flip every non-terminal child (ready/in_progress) to the same target status. Required when the epic has any non-terminal children — otherwise the call is rejected." },
        },
        required: ["threadId", "itemId"],
      },
      handler: (args: {
        streamId?: string;
        threadId: string;
        itemId: string;
        parentId?: string | null;
        title?: string;
        description?: string;
        acceptanceCriteria?: string | null;
        status?: WorkItemStatus;
        priority?: WorkItemPriority;
        touchedFiles?: string[];
        cascade?: boolean;
      }) => {
        resolveThreadAndStream(args);
        if (markFiledThisTurn) markFiledThisTurn(args.threadId);
        // Epic-cascade guard mirrors the one on complete_task: only
        // applies when the status transition is to done or blocked
        // (an "epic close"). Other transitions (e.g. epic → ready,
        // epic → in_progress) pass through unchanged.
        if (args.status === "done" || args.status === "blocked") {
          const epicGuardError = checkEpicCascadeGuard(workItemStore, args.threadId, args.itemId, args.cascade);
          if (epicGuardError) return epicGuardError;
          if (args.cascade) {
            cascadeChildrenStatus(workItemStore, args.threadId, args.itemId, args.status as "done" | "blocked");
          }
        }
        const item = workItemStore.updateItem({
          threadId: args.threadId,
          itemId: args.itemId,
          parentId: args.parentId,
          title: args.title,
          description: args.description,
          acceptanceCriteria: args.acceptanceCriteria,
          status: args.status,
          priority: args.priority,
          touchedFiles: args.touchedFiles,
          actorKind: "agent",
          actorId: "mcp",
        });
        return { ok: true, id: item.id, status: item.status };
      },
    },
    {
      name: "oxplow__transition_work_items",
      description:
        "Flip the status of multiple work items in one call. Useful at phase boundaries " +
        "(e.g., moving several subtasks from `ready` to `in_progress`, or rolling an epic " +
        "and its children to `done` together). Each transition fires the same side " +
        "effects as an individual `update_work_item` call — effort open/close, work-item.changed " +
        "events, audit log entries — so downstream subscribers don't need to special-case batching.",
      inputSchema: {
        type: "object",
        properties: {
          transitions: {
            type: "array",
            description: "List of status transitions to apply. Each entry names one item and its new status.",
            items: {
              type: "object",
              properties: {
                threadId: { type: "string", description: "Thread that owns the item. Usually the same for every entry." },
                itemId: { type: "string", description: "Id of the item to transition." },
                status: { type: "string", description: "New status. Same enum as update_work_item.status." },
              },
              required: ["threadId", "itemId", "status"],
            },
          },
        },
        required: ["transitions"],
      },
      handler: (args: {
        transitions: Array<{ threadId: string; itemId: string; status: WorkItemStatus }>;
      }) => {
        if (!Array.isArray(args.transitions) || args.transitions.length === 0) {
          throw new Error("transition_work_items: `transitions` must be a non-empty array");
        }
        // Resolve each thread once up front so an invalid threadId fails the
        // whole call before any side effects. Cache validated threads.
        const validatedThreads = new Set<string>();
        for (const t of args.transitions) {
          if (!validatedThreads.has(t.threadId)) {
            resolveThreadAndStream({ threadId: t.threadId });
            validatedThreads.add(t.threadId);
            if (markFiledThisTurn) markFiledThisTurn(t.threadId);
          }
        }
        const results: Array<{ id: string; status: WorkItemStatus }> = [];
        for (const t of args.transitions) {
          const item = workItemStore.updateItem({
            threadId: t.threadId,
            itemId: t.itemId,
            status: t.status,
            actorKind: "agent",
            actorId: "mcp",
          });
          results.push({ id: item.id, status: item.status });
        }
        return { ok: true, results };
      },
    },
    {
      name: "oxplow__get_work_item",
      description: "Fetch one work item plus its links (incoming + outgoing) and recent audit events. Use when resuming work on an item and you need the full context without pulling the whole thread.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id." },
          itemId: { type: "string", description: "Required id of the work item to fetch." },
        },
        required: ["threadId", "itemId"],
      },
      handler: (args: { streamId?: string; threadId: string; itemId: string }) => {
        resolveThreadAndStream(args);
        const detail = workItemStore.getItemDetail(args.threadId, args.itemId, 5);
        if (!detail) throw new Error(`unknown work item: ${args.itemId}`);
        return {
          ...detail,
          recentEvents: detail.recentEvents.map(slimWorkItemEvent),
        };
      },
    },
    {
      name: "oxplow__delete_work_item",
      description: "Soft-delete a work item. Hidden from lists but preserved in the audit log.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          itemId: { type: "string", description: "Required id of the work item to delete." },
        },
        required: ["threadId", "itemId"],
      },
      handler: (args: { streamId?: string; threadId: string; itemId: string }) => {
        resolveThreadAndStream(args);
        workItemStore.deleteItem(args.threadId, args.itemId, "agent", "mcp");
        return { ok: true };
      },
    },
    {
      name: "oxplow__reorder_work_items",
      description: "Reorder sibling work items within a thread. All ids must share the same parent.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          orderedItemIds: {
            type: "array",
            description: "Full list of sibling work item ids in the desired new order.",
            items: { type: "string" },
          },
        },
        required: ["threadId", "orderedItemIds"],
      },
      handler: (args: { streamId?: string; threadId: string; orderedItemIds: string[] }) => {
        resolveThreadAndStream(args);
        workItemStore.reorderItems(args.threadId, args.orderedItemIds, "agent", "mcp");
        return { ok: true };
      },
    },
    {
      name: "oxplow__link_work_items",
      description:
        "Create a relationship between two work items in one thread. linkType is one of: " +
        "`blocks` (from-item must finish before to-item starts), " +
        "`relates_to` (general association), " +
        "`discovered_from` (from-item was uncovered while working on to-item; preferred for scope-creep escape), " +
        "`duplicates` (from-item is the same work as to-item — close the dupe), " +
        "`supersedes` (from-item replaces to-item — the older one is stale), " +
        "`replies_to` (from-item is a threaded note/response to to-item).",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          fromItemId: { type: "string", description: "Source work item id." },
          toItemId: { type: "string", description: "Target work item id." },
          linkType: { type: "string", description: "One of blocks, relates_to, discovered_from, duplicates, supersedes, replies_to." },
        },
        required: ["threadId", "fromItemId", "toItemId", "linkType"],
      },
      handler: (args: {
        streamId?: string;
        threadId: string;
        fromItemId: string;
        toItemId: string;
        linkType: "blocks" | "relates_to" | "discovered_from" | "duplicates" | "supersedes" | "replies_to";
      }) => {
        resolveThreadAndStream(args);
        workItemStore.linkItems(args.threadId, args.fromItemId, args.toItemId, args.linkType);
        return { ok: true };
      },
    },
    {
      name: "oxplow__add_work_note",
      description: "Append a note/history entry to a work item in one thread. Always pass the threadId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted; only passed explicitly for the no-threadId form of get_thread_context." },
          threadId: { type: "string", description: "Required thread id for the work you are managing." },
          itemId: { type: "string", description: "Optional work item id if the note belongs to one specific item." },
          note: { type: "string", description: "Required note content." },
        },
        required: ["threadId", "note"],
      },
      handler: (args: { streamId?: string; threadId: string; itemId?: string; note: string }) => {
        resolveThreadAndStream(args);
        workItemStore.addNote(args.threadId, args.itemId ?? null, args.note, "agent", "mcp");
        return { ok: true };
      },
    },
    {
      name: "oxplow__fork_thread",
      description:
        "Create a new thread on the same stream as `sourceThreadId`, seeded with a single " +
        "`note`-kind work item carrying the `summary` you supply as context. Optionally moves " +
        "the listed `moveItemIds` across (each must currently be `ready` or `blocked` on the " +
        "source thread — in-progress / terminal items are rejected). Returns " +
        "`{ newThreadId }`. The new thread starts queued (never auto-writer); promote it " +
        "explicitly if you want it to take over the worktree.",
      inputSchema: {
        type: "object",
        properties: {
          sourceThreadId: { type: "string", description: "Thread to fork from." },
          title: { type: "string", description: "Title for the new thread." },
          summary: { type: "string", description: "Carry-over context — stored as the description of a single seeded `note` work item on the new thread." },
          moveItemIds: {
            type: "array",
            description: "Optional list of work item ids to move from source to new thread. Each must currently be `ready` or `blocked`.",
            items: { type: "string" },
          },
        },
        required: ["sourceThreadId", "title", "summary"],
      },
      handler: (args: {
        sourceThreadId: string;
        title: string;
        summary: string;
        moveItemIds?: string[];
      }) => {
        if (!forkThread) throw new Error("oxplow__fork_thread: runtime not wired");
        const result = forkThread({
          sourceThreadId: args.sourceThreadId,
          title: args.title,
          summary: args.summary,
          moveItemIds: args.moveItemIds,
        });
        return { ok: true, newThreadId: result.newThreadId };
      },
    },
    {
      name: "oxplow__delegate_query",
      description:
        "Prepare an exploration query for an Explore subagent. Use when you need to understand a codebase area before dispatching real work and would otherwise read 5+ files inline — offloading the reads keeps your own cached context small. " +
        "Returns `{ prompt, provisionalNoteId }`. The orchestrator then calls `Agent(subagent_type='Explore', prompt=<prompt>)`; the prompt already instructs the subagent to call `oxplow__record_query_finding({ noteId: <provisionalNoteId>, body })` with its findings. Read the finding later via `oxplow__get_thread_notes` only when you need the content.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id — the finding note is scoped to this thread." },
          question: { type: "string", description: "The exploration question to answer. Be specific about what you need to know." },
          focus: { type: "string", description: "Optional focus hint (file paths, module names, areas of the codebase) to narrow the subagent's search." },
        },
        required: ["threadId", "question"],
      },
      handler: (args: { streamId?: string; threadId: string; question: string; focus?: string }) => {
        resolveThreadAndStream(args);
        const question = String(args.question ?? "").trim();
        if (!question) throw new Error("oxplow__delegate_query: `question` is required");
        const focus = typeof args.focus === "string" ? args.focus.trim() : "";
        const provisionalNoteId = workItemStore.addThreadNote(args.threadId, "", "explore-subagent");
        const prompt = composeDelegateQueryPrompt({
          threadId: args.threadId,
          question,
          focus,
          noteId: provisionalNoteId,
        });
        return { ok: true, prompt, provisionalNoteId };
      },
    },
    {
      name: "oxplow__record_query_finding",
      description:
        "Write the Explore subagent's finding into a pre-allocated thread-scoped note (id returned by `oxplow__delegate_query`). Call this once at the end of the exploration — the orchestrator reads it later via `oxplow__get_thread_notes`.",
      inputSchema: {
        type: "object",
        properties: {
          noteId: { type: "string", description: "Provisional thread-note id returned by `oxplow__delegate_query`." },
          body: { type: "string", description: "Finding content. Structured prose is fine — it gets stored verbatim." },
        },
        required: ["noteId", "body"],
      },
      handler: (args: { noteId: string; body: string }) => {
        if (!args.noteId) throw new Error("oxplow__record_query_finding: `noteId` is required");
        if (typeof args.body !== "string") {
          throw new Error("oxplow__record_query_finding: `body` must be a string");
        }
        workItemStore.updateThreadNoteBody(args.noteId, args.body);
        return { ok: true, noteId: args.noteId };
      },
    },
    {
      name: "oxplow__get_thread_notes",
      description:
        "Return recent thread-scoped notes (reverse chronological). Thread-scoped notes are findings from `oxplow__delegate_query` Explore subagents and any other thread-level context not attached to a specific work item.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id." },
          limit: { type: "number", description: "Optional cap on rows returned (default 5, max 100)." },
        },
        required: ["threadId"],
      },
      handler: (args: { streamId?: string; threadId: string; limit?: number }) => {
        resolveThreadAndStream(args);
        const notes = workItemStore.listThreadNotes(args.threadId, args.limit ?? 5);
        return { notes };
      },
    },
    {
      name: "oxplow__add_followup",
      description:
        "Stash a transient follow-up reminder for the current thread. Use when you defer a sub-ask " +
        "mid-turn that doesn't warrant a full work item — just \"I'll get back to that next\" within " +
        "the same conversation. Returns `{ ok: true, id }`. Surfaces as an italic muted reminder line " +
        "at the top of the To Do section in the Work panel; transient (in-memory only, lost on " +
        "runtime restart). When you handle the follow-up, call `oxplow__remove_followup` with the id " +
        "in the same turn. NEVER add a follow-up alongside an actual work item for the same concern — " +
        "if the deferred ask warrants a row the user reviews/accepts, file a task instead.",
      inputSchema: {
        type: "object",
        properties: {
          threadId: { type: "string", description: "Required thread id the follow-up is scoped to." },
          note: { type: "string", description: "Required short reminder text — what you'll come back to. Trimmed; empty notes are rejected." },
        },
        required: ["threadId", "note"],
      },
      handler: (args: { threadId: string; note: string }) => {
        if (!followupStore) throw new Error("oxplow__add_followup: runtime not wired");
        resolveThreadAndStream({ threadId: args.threadId });
        const entry = followupStore.add(args.threadId, args.note);
        return { ok: true, id: entry.id };
      },
    },
    {
      name: "oxplow__remove_followup",
      description:
        "Dismiss a transient follow-up by id. Call this in the same turn you actually handle the " +
        "deferred ask, so the To Do reminder line disappears from the Work panel.",
      inputSchema: {
        type: "object",
        properties: {
          threadId: { type: "string", description: "Required thread id." },
          id: { type: "string", description: "Required follow-up id (returned by add_followup)." },
        },
        required: ["threadId", "id"],
      },
      handler: (args: { threadId: string; id: string }) => {
        if (!followupStore) throw new Error("oxplow__remove_followup: runtime not wired");
        resolveThreadAndStream({ threadId: args.threadId });
        const ok = followupStore.remove(args.threadId, args.id);
        return { ok };
      },
    },
    {
      name: "oxplow__list_followups",
      description: "List the current thread's transient follow-up reminders in insertion order.",
      inputSchema: {
        type: "object",
        properties: {
          threadId: { type: "string", description: "Required thread id." },
        },
        required: ["threadId"],
      },
      handler: (args: { threadId: string }) => {
        if (!followupStore) throw new Error("oxplow__list_followups: runtime not wired");
        resolveThreadAndStream({ threadId: args.threadId });
        return { followups: followupStore.list(args.threadId) };
      },
    },
    {
      name: "oxplow__await_user",
      description:
        "Signal that you are explicitly waiting on the user to answer a question or make a decision before continuing. " +
        "Call this when your reply ends with a real clarifying question, an A/B/C choice, or any other ask where the user's " +
        "next message is needed before more work happens. Sets a per-turn flag the Stop hook reads: while set, the next Stop " +
        "allows-stop and suppresses every directive (commit, audit, ready-work, filing-enforcement) so the agent does not " +
        "march onto the next queue item past your open question. The flag is cleared automatically when the user sends a " +
        "fresh prompt. Do NOT call this for rhetorical asides or status updates — only for genuine open questions.",
      inputSchema: {
        type: "object",
        properties: {
          threadId: { type: "string", description: "Required thread id." },
          question: {
            type: "string",
            description:
              "Short summary of what you are waiting on the user for — a sentence or two. Stored on the turn for telemetry / future UI surfacing; does not need to match your chat reply verbatim.",
          },
        },
        required: ["threadId", "question"],
      },
      handler: (args: { threadId: string; question: string }) => {
        resolveThreadAndStream({ threadId: args.threadId });
        if (markAwaitingUser) markAwaitingUser(args.threadId, args.question);
        return { ok: true };
      },
    },
    {
      name: "oxplow__get_subsystem_doc",
      description:
        "Return the contents of a `.context/<name>.md` doc from the stream's worktree. Cheap alternative to a Read call when you only need the durable subsystem knowledge — saves the model from re-reading the same .context doc 20+ times per session. Returns `{ name, path, content, exists }`. When the doc doesn't exist, `exists` is false and `content` is empty (no error) so the caller can branch on it instead of seeing a hard file-not-found.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from threadId when omitted." },
          threadId: { type: "string", description: "Required thread id (used to find the stream's worktree)." },
          name: {
            type: "string",
            description: "Doc name without the `.md` suffix or `.context/` prefix. Examples: \"data-model\", \"agent-model\", \"ipc-and-stores\", \"theming\", \"git-integration\", \"editor-and-monaco\", \"code-quality\", \"architecture\", \"usability\".",
          },
        },
        required: ["threadId", "name"],
      },
      handler: (args: { streamId?: string; threadId: string; name: string }) => {
        const { stream } = resolveThreadAndStream(args);
        const safeName = String(args.name || "").trim();
        // Reject any path-traversal / directory chars so the tool can't be
        // used to read arbitrary files outside `.context/`.
        if (!safeName || /[\\/]|\.\./.test(safeName)) {
          throw new Error("get_subsystem_doc: `name` must be a bare doc name (no slashes, no ..)");
        }
        const fs = require("node:fs") as typeof import("node:fs");
        const path = require("node:path") as typeof import("node:path");
        const relPath = path.join(".context", `${safeName}.md`);
        const absPath = path.join(stream.worktree_path, relPath);
        if (!fs.existsSync(absPath)) {
          return { name: safeName, path: relPath, content: "", exists: false };
        }
        const content = fs.readFileSync(absPath, "utf8");
        return { name: safeName, path: relPath, content, exists: true };
      },
    },
  ];
}
