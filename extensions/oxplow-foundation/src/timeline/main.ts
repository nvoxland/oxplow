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
  truncated: boolean;
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

/** Each kind's rows in the window `?1`..`?2` (`?3`: one thread's), as
 *  `kind, ref, label, start_at, end_at`: efforts, sessions and threads
 *  that started before it ends and hadn't ended by its start, commits in
 *  it. One query a kind, so one busy kind can't crowd out the others
 *  under the row cap, newest first, so a capped kind keeps the latest. */
const QUERIES: Record<string, string> = {
  effort: `SELECT 'effort', ref, COALESCE(NULLIF(title, ''), ref), started_at, ended_at
             FROM v_effort
            WHERE started_at < ?2 AND COALESCE(ended_at, ?2) > ?1 AND (?3 IS NULL OR thread_id = ?3)
            ORDER BY started_at DESC`,
  session: `SELECT 'session', ref, COALESCE(display_name, title, harness), opened_at, closed_at
              FROM v_agent_session
             WHERE opened_at < ?2 AND COALESCE(closed_at, ?2) > ?1 AND (?3 IS NULL OR thread_id = ?3)
             ORDER BY opened_at DESC`,
  thread: `SELECT 'thread', ref, title, created_at, closed_at
             FROM v_thread
            WHERE created_at < ?2 AND COALESCE(closed_at, ?2) > ?1
              AND (?3 IS NULL OR ref = 'thread:thr' || ?3)
            ORDER BY created_at DESC`,
  commit: `SELECT 'commit', ref, subject, committed_at, NULL
             FROM v_commit
            WHERE committed_at >= ?1 AND committed_at < ?2 AND ?3 IS NULL
            ORDER BY committed_at DESC`,
};

/** The many-per-day kinds, counted per `?4` (`day` or `week`, UTC) from
 *  where each starts in the window: a wide window draws these counts, not
 *  rows, so no cap leaves gaps. */
const bucket = (column: string) =>
  `CASE ?4 WHEN 'day' THEN date(max(${column}, ?1)) ELSE date(max(${column}, ?1), '-6 days', 'weekday 1') END`;
const COUNTS: Record<string, string> = {
  effort: `SELECT ${bucket("started_at")} AS bucket, count(*)
             FROM v_effort
            WHERE started_at < ?2 AND COALESCE(ended_at, ?2) > ?1 AND (?3 IS NULL OR thread_id = ?3)
            GROUP BY bucket`,
  commit: `SELECT ${bucket("committed_at")} AS bucket, count(*)
             FROM v_commit
            WHERE committed_at >= ?1 AND committed_at < ?2 AND ?3 IS NULL
            GROUP BY bucket`,
};
/** Above this many in the window, a kind draws as counts. */
const DETAIL = 300;

const GROUPS = [
  { id: "thread", content: "Threads" },
  { id: "session", content: "Sessions" },
  { id: "effort", content: "Efforts" },
  { id: "commit", content: "Commits" },
];
const NOUNS: Record<string, string> = { thread: "threads", session: "sessions", effort: "efforts", commit: "commits" };

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
  const buckets = new Map<string, { start: Date; end: Date }>();
  const container = document.getElementById("timeline")!;
  const note = document.getElementById("note")!;
  const timeline = new Timeline(container, items, GROUPS, {
    start,
    end,
    height: "100%",
    verticalScroll: true,
    stack: true,
    zoomMin: 60 * 60 * 1000,
    orientation: { axis: "top", item: "top" },
    margin: { item: { horizontal: 4, vertical: 4 } },
    // Items too close to tell apart at this zoom draw as one ("12
    // efforts"); zooming in takes them apart.
    cluster: {
      maxItems: 1,
      // Only the many short ones; a thread or session span reads alone.
      clusterCriteria: (a: { group?: string; id?: string }, b: { group?: string; id?: string }) =>
        a.group === b.group &&
        (a.group === "effort" || a.group === "commit") &&
        !buckets.has(String(a.id)) &&
        !buckets.has(String(b.id)),
      titleTemplate: "{count} items — zoom in to see them",
      fitOnDoubleClick: true,
    },
    tooltip: { followMouse: true, delay: 300 },
  } as never);

  let loading = 0;
  const load = async (from: Date, to: Date) => {
    const mine = ++loading;
    const window = [from.toISOString(), to.toISOString(), thread];
    const size = to.getTime() - from.getTime() > 45 * DAY ? "week" : "day";
    // A countable kind's counts first; its rows only when there are few.
    const results = await Promise.all(
      Object.entries(QUERIES).map(async ([kind, sql]) => {
        const counts = COUNTS[kind] ? (await component.querySql(COUNTS[kind], [...window, size])).rows : null;
        const total = counts?.reduce((sum, row) => sum + Number(row[1]), 0) ?? 0;
        if (counts && total > DETAIL) return { kind, counts, total, rows: null };
        return { kind, counts: null, total, rows: await component.querySql(sql, window) };
      }),
    );
    if (mine !== loading) return;
    items.clear();
    buckets.clear();
    const length = size === "week" ? 7 * DAY : DAY;
    const noun = (kind: string) => NOUNS[kind];
    const counted = results.flatMap((r) =>
      (r.counts ?? []).map((row) => {
        const start = new Date(`${row[0]}T00:00:00Z`);
        const id = `count:${r.kind}:${row[0]}`;
        buckets.set(id, { start, end: new Date(start.getTime() + length) });
        const label = `${row[1]} ${noun(r.kind)} that ${size}`;
        return {
          id,
          group: r.kind,
          content: String(row[1]),
          title: `${label} — click to zoom in`,
          start,
          // A gap to the next count wider than the item margin, so a row of
          // counts stays one row.
          end: new Date(start.getTime() + length * 0.8),
          type: "range",
          className: `kind-${r.kind} count`,
        };
      }),
    );
    const all = results.flatMap((r) => r.rows?.rows ?? []);
    items.add(counted);
    items.add(
      all.map((row) => {
        const kind = String(row[0]);
        const ref = String(row[1]);
        const label = String(row[2] ?? ref);
        const startAt = new Date(String(row[3]));
        const endRaw = row[4];
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
    // Laid out against the frame's size now, not the one it had when the
    // timeline was made (it can start before the page has its size).
    timeline.redraw();
    const capped = results.filter((r) => r.rows?.truncated).map((r) => noun(r.kind));
    const countedKinds = results.filter((r) => r.counts).map((r) => noun(r.kind));
    note.textContent =
      all.length === 0 && counted.length === 0
        ? "Nothing happened in this range."
        : [
            countedKinds.length > 0
              ? `${countedKinds.join(" and ")} are counted per ${size} — zoom in to see each one.`.replace(/^./, (c) => c.toUpperCase())
              : "",
            capped.length > 0
              ? `Showing the latest ${results.find((r) => r.rows?.truncated)?.rows?.rows.length} ${capped.join(" and ")} in this range — zoom in for the earlier ones.`
              : "",
          ]
            .filter(Boolean)
            .join(" ");
    void component.setTitle(`Timeline: ${shortDate(from)} – ${shortDate(to)}`);
  };

  // The frame's size settles after it loads, and changes with the window.
  new ResizeObserver(() => timeline.redraw()).observe(container);

  // A load that fails says so, under the timeline.
  const show = (from: Date, to: Date) =>
    load(from, to).catch((e: unknown) => {
      note.textContent = `Couldn't load this range: ${(e as { message?: string })?.message ?? String(e)}`;
    });
  let timer: ReturnType<typeof setTimeout> | undefined;
  timeline.on("rangechanged", (event: { start: Date; end: Date; byUser: boolean }) => {
    clearTimeout(timer);
    timer = setTimeout(() => void show(event.start, event.end), 250);
  });
  timeline.on("click", (event: { item: string | null; event: MouseEvent }) => {
    if (!event.item) return;
    const counted = buckets.get(event.item);
    if (counted) {
      timeline.setWindow(counted.start, counted.end);
      return;
    }
    void component.navigate(event.item, { newTab: event.event.metaKey || event.event.ctrlKey });
  });

  // Everything loaded, with room past both ends: a span that starts near
  // now still shows its label (the timeline's own fit ends at the last end).
  const fit = () => {
    let first = Infinity;
    let last = -Infinity;
    items.forEach((item) => {
      first = Math.min(first, item.start.getTime());
      last = Math.max(last, (item.end ?? item.start).getTime());
    });
    if (first > last) return;
    const pad = Math.max((last - first) * 0.08, 60 * 60 * 1000);
    timeline.setWindow(new Date(first - pad), new Date(last + pad));
  };

  void component.setActions([
    { id: "today", label: "Today" },
    { id: "fit", label: "Fit" },
  ]);
  component.onAction((id) => {
    if (id === "today") timeline.setWindow(new Date(Date.now() - DAY / 2), new Date(Date.now() + DAY / 2));
    if (id === "fit") fit();
  });

  void show(start, end);
});
