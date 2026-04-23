import { useEffect, useMemo, useState } from "react";
import { listOpenTurns, subscribeTurnEvents, type AgentTurn } from "../../api.js";
import { reportUiError } from "../../ui-error.js";

/**
 * Renders the live agent turns for a thread as synthetic rows at the top
 * of the In progress section. Passively tracks `agent_turn` rows where
 * `ended_at IS NULL` AND `started_at >= runtime.startedAt`. When the turn
 * closes, the row disappears — no status flips, no notes, no cleanup.
 *
 * Each row shows:
 *   - A neutral glyph (matches the muted-text style of other row icons,
 *     not the colored hourglass emoji)
 *   - The prompt, truncated
 *   - If `task_list_json` is populated, a collapsible TaskCreate breakdown
 *     (collapsed by default once ≥4 steps exist).
 *
 * When no turns are open and `parentSectionEmpty` is true, renders a
 * "(waiting)" placeholder so the In progress section has an explicit
 * idle indicator instead of appearing blank.
 */
export function OpenTurnsList({
  streamId,
  threadId,
  parentSectionEmpty,
}: {
  streamId: string | null;
  threadId: string | null;
  parentSectionEmpty: boolean;
}) {
  const [turns, setTurns] = useState<AgentTurn[]>([]);

  useEffect(() => {
    if (!threadId) { setTurns([]); return; }
    let cancelled = false;
    const refresh = () => {
      listOpenTurns(threadId)
        .then((rows) => { if (!cancelled) setTurns(rows); })
        .catch((err) => reportUiError("Load open turns", err));
    };
    refresh();
    const off = subscribeTurnEvents(streamId ?? "all", (event) => {
      if (event.threadId !== threadId) return;
      refresh();
    });
    return () => { cancelled = true; off(); };
  }, [streamId, threadId]);

  if (turns.length === 0) {
    if (!parentSectionEmpty) return null;
    return (
      <div
        data-testid="plan-open-turns-waiting"
        style={{ padding: "4px 10px", fontSize: 11, color: "var(--muted)", fontStyle: "italic" }}
      >
        (waiting)
      </div>
    );
  }
  return (
    <div data-testid="plan-open-turns">
      {turns.map((turn) => (
        <OpenTurnRow key={turn.id} turn={turn} />
      ))}
    </div>
  );
}

const BRAILLE_SPINNER_FRAMES = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

function BrailleSpinner() {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    const id = setInterval(() => {
      setFrame((prev) => (prev + 1) % BRAILLE_SPINNER_FRAMES.length);
    }, 90);
    return () => clearInterval(id);
  }, []);
  return (
    <span
      aria-hidden="true"
      title="Turn in progress"
      style={{
        color: "var(--muted)",
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        width: 12,
        display: "inline-block",
        textAlign: "center",
      }}
    >
      {BRAILLE_SPINNER_FRAMES[frame]}
    </span>
  );
}

function OpenTurnRow({ turn }: { turn: AgentTurn }) {
  const promptPreview = truncate(turn.prompt, 80);
  const tasks = useMemo(() => parseTasks(turn.task_list_json), [turn.task_list_json]);
  const [expanded, setExpanded] = useState(() => tasks.length > 0 && tasks.length < 4);
  return (
    <div
      data-testid="plan-open-turn-row"
      style={{
        padding: "4px 8px",
        fontSize: 12,
      }}
    >
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <BrailleSpinner />
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {promptPreview}
        </span>
      </div>
      {tasks.length > 0 ? (
        <div style={{ marginTop: 4 }}>
          <button
            type="button"
            onClick={() => setExpanded((prev) => !prev)}
            style={{
              background: "transparent", border: "none", cursor: "pointer",
              padding: "2px 0", color: "var(--muted)", fontSize: 11,
            }}
            data-testid="plan-open-turn-tasks-toggle"
          >
            {expanded ? "▼" : "▶"} {tasks.length} step{tasks.length === 1 ? "" : "s"}
          </button>
          {expanded ? (
            <ul style={{ listStyle: "none", margin: 0, padding: "0 0 0 16px" }}>
              {tasks.map((t, i) => (
                <li key={i} style={{ fontSize: 11, color: "var(--fg)" }}>
                  <span style={{ marginRight: 6 }}>{taskGlyph(t.status)}</span>
                  {t.content}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function taskGlyph(status: string): string {
  if (status === "completed") return "☑";
  if (status === "in_progress") return "▶";
  return "☐";
}

function parseTasks(json: string | null): Array<{ content: string; status: string }> {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((e): e is { content: string; status: string } =>
      e != null && typeof e === "object"
        && typeof (e as { content?: unknown }).content === "string"
        && typeof (e as { status?: unknown }).status === "string",
    );
  } catch {
    return [];
  }
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return s.slice(0, n - 1) + "…";
}

