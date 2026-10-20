// The Timeline page (oxplow-foundation, `pages: [{ id: timeline, component:
// timeline }]`): efforts, agent sessions, threads and commits on a
// zoomable time axis. A custom page: it reads its rows with `querySql`
// (the component declares `sql: read`), titles its page with the range,
// puts Today and Fit in the page's bar, and opens what is clicked.
import { DataSet, Timeline } from "vis-timeline/standalone";

type Cell = string | number | boolean | null;
interface Rows {
  columns: string[];
  rows: Cell[][];
}
interface Component {
  page: { params: Record<string, string>; streamId: string | null; threadId: string | null } | null;
  querySql(sql: string, params?: Cell[]): Promise<Rows>;
  navigate(ref: string, options?: { newTab?: boolean }): Promise<null>;
  setTitle(title: string): Promise<null>;
  setActions(actions: { id: string; label: string; primary?: boolean }[]): Promise<null>;
  onAction(listener: (id: string) => void): () => void;
  applyTheme(): void;
}
declare const oxplow: { connect(): Promise<Component> };

const DAY = 24 * 60 * 60 * 1000;

/** Everything that started before `?2` and hadn't ended by `?1`: efforts,
 *  sessions and threads as spans, commits as points; `?3` one thread's. */
const SQL = `
SELECT kind, ref, label, start_at, end_at FROM (
  SELECT 'effort' AS kind, ref, COALESCE(NULLIF(title, ''), ref) AS label,
         started_at AS start_at, ended_at AS end_at, thread_id
    FROM v_effort WHERE started_at < ?2 AND COALESCE(ended_at, ?2) > ?1
  UNION ALL
  SELECT 'session', ref, COALESCE(display_name, title, harness), opened_at, closed_at, thread_id
    FROM v_agent_session WHERE opened_at < ?2 AND COALESCE(closed_at, ?2) > ?1
  UNION ALL
  SELECT 'thread', ref, title, created_at, closed_at, CAST(substr(ref, 11) AS INTEGER)
    FROM v_thread WHERE created_at < ?2 AND COALESCE(closed_at, ?2) > ?1
  UNION ALL
  SELECT 'commit', ref, subject, committed_at, NULL, NULL
    FROM v_commit WHERE committed_at >= ?1 AND committed_at < ?2 AND ?3 IS NULL
) WHERE ?3 IS NULL OR thread_id = ?3
ORDER BY start_at`;

const GROUPS = [
  { id: "thread", content: "Threads" },
  { id: "session", content: "Sessions" },
  { id: "effort", content: "Efforts" },
  { id: "commit", content: "Commits" },
];

const dateOf = (raw: string | undefined, fallback: number) => {
  const t = raw ? Date.parse(raw) : NaN;
  return Number.isNaN(t) ? new Date(fallback) : new Date(t);
};

const shortDate = (d: Date) => d.toLocaleDateString(undefined, { month: "short", day: "numeric" });

oxplow.connect().then((component) => {
  component.applyTheme();
  const params = component.page?.params ?? {};
  const now = Date.now();
  const start = dateOf(params.from, now - 7 * DAY);
  const end = dateOf(params.to, now + DAY / 4);
  // `thread` is a thread id (`thr3`) or its number.
  const threadParam = params.thread?.replace(/^thread:/, "").replace(/^thr/, "");
  const thread = threadParam && /^\d+$/.test(threadParam) ? Number(threadParam) : null;

  const items = new DataSet<{ id: string; group: string; content: string; start: Date; end?: Date; type: string; className: string; title: string }>();
  const container = document.getElementById("timeline")!;
  const empty = document.getElementById("empty")!;
  const timeline = new Timeline(container, items, GROUPS, {
    start,
    end,
    stack: true,
    zoomMin: 60 * 60 * 1000,
    orientation: { axis: "top" },
    margin: { item: 4 },
  });

  let loading = 0;
  const load = async (from: Date, to: Date) => {
    const mine = ++loading;
    const rows = await component.querySql(SQL, [from.toISOString(), to.toISOString(), thread]);
    if (mine !== loading) return;
    const at = (name: string) => rows.columns.indexOf(name);
    items.clear();
    items.add(
      rows.rows.map((row) => {
        const kind = String(row[at("kind")]);
        const ref = String(row[at("ref")]);
        const label = String(row[at("label")] ?? ref);
        const startAt = new Date(String(row[at("start_at")]));
        const endRaw = row[at("end_at")];
        const point = kind === "commit";
        return {
          id: ref,
          group: kind,
          content: label,
          title: label,
          start: startAt,
          // An open span runs to now.
          end: point ? undefined : endRaw ? new Date(String(endRaw)) : new Date(),
          type: point ? "point" : "range",
          className: `kind-${kind}`,
        };
      }),
    );
    empty.hidden = rows.rows.length > 0;
    void component.setTitle(`Timeline: ${shortDate(from)} – ${shortDate(to)}`);
  };

  let timer: ReturnType<typeof setTimeout> | undefined;
  timeline.on("rangechanged", (event: { start: Date; end: Date; byUser: boolean }) => {
    clearTimeout(timer);
    timer = setTimeout(() => void load(event.start, event.end), 250);
  });
  timeline.on("click", (event: { item: string | null; event: MouseEvent }) => {
    if (!event.item) return;
    void component.navigate(event.item, { newTab: event.event.metaKey || event.event.ctrlKey });
  });

  void component.setActions([
    { id: "today", label: "Today" },
    { id: "fit", label: "Fit" },
  ]);
  component.onAction((id) => {
    if (id === "today") timeline.setWindow(new Date(Date.now() - DAY / 2), new Date(Date.now() + DAY / 2));
    if (id === "fit") timeline.fit();
  });

  void load(start, end);
});
