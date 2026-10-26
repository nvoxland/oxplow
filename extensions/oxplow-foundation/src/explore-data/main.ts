// Explore Data (oxplow-foundation's `explore-data` page, whose view is
// this component): an editable SQL box over oxplow's models — metrics
// too, through `metric_grid()` — with a viz picker. oxplow draws the
// result under it with its own components (`showView`: an inline page of
// the query), live, with Save as Page and Pin to Dashboard; the catalog is
// the page's rail, whose pick sets the page's `model` param (heard as an
// update's `params`). What it shows is its view (the tab's address):
// `sql`, `viz`, `chart`, `template` (and `model`, as it last picked one);
// the SQL being edited, not yet run, is its history state.
import {
  CHART_ROLES,
  chartDefaults,
  chartFits,
  DIMENSIONS_SQL,
  lineage,
  LINEAGE_SQL,
  METRICS_SQL,
  metricTemplate,
  MODEL_COLUMNS_SQL,
  MODEL_SQL,
  modelColumns,
  readView,
  rowObjects,
  sampleSql,
  sliceTemplate,
  specOf,
  templateParam,
  VIZ_OPTIONS,
  type Cell,
  type Chart,
  type ExploreView,
  type MetricTemplate,
  type Rows,
  type Viz,
} from "./model.js";

interface Refusal {
  code?: string;
  message?: string;
  field?: string;
}
interface Component {
  viewParams: Record<string, string> | null;
  historyState: Record<string, unknown> | null;
  setViewParams(params: Record<string, string | null>): Promise<null>;
  onViewParams(listener: (params: Record<string, string>) => void): () => void;
  setHistoryState(patch: Record<string, unknown>): Promise<null>;
  querySql(sql: string, params?: Cell[]): Promise<Rows>;
  invoke(command: string, input?: unknown): Promise<unknown>;
  setTitle(title: string): Promise<null>;
  /** Draw an inline page (its YAML) under the explorer; its first loader's rows. */
  showView(spec: string | null): Promise<Rows | null>;
  /** The page's params: `model`, as the rail picked it. */
  params: Record<string, Cell>;
  onUpdate(listener: (update: { params: Record<string, Cell> }) => void): () => void;
  applyTheme(): void;
}
declare const oxplow: { connect(): Promise<Component> };

/** What "ask your agent" starts the prompt with (the draft command's own
 *  `ui.input.text`). */
const AGENT_PROMPT = "Build me an oxplow page (one I can open with Cmd+P) that shows ";

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> & { testId?: string } = {}, ...children: (Node | string)[]) => {
  const node = document.createElement(tag);
  const { testId, ...rest } = props;
  Object.assign(node, rest);
  if (testId) node.dataset.testid = testId;
  node.append(...children);
  return node;
};

void oxplow.connect().then((c) => {
  c.applyTheme();
  void c.setTitle("Explore Data");
  let view: ExploreView = readView(c.viewParams ?? {});
  // The model the rail picked: the page's `model` param.
  const pageModel = (params: Record<string, Cell>) => (typeof params.model === "string" ? params.model : "");
  let picked = pageModel(c.params ?? {});
  // The columns of what was drawn last: the chart's pickers offer them.
  let columns: string[] = [];

  // ---- The page ----
  const intro = el(
    "p",
    { className: "ox-intro" },
    "Everything oxplow knows, as read-only SQL views. Pick one on the right, tweak the query, and save it as a page to keep it. Metrics read as columns of a grid: ",
    el("code", {}, "SELECT bucket, MEASURE('oxplow.coverage.abs_pct') FROM metric_grid('week')"),
    ". Or ",
  );
  const ask = el("button", { type: "button", className: "ox-link", testId: "explore-new-lens" }, "ask your agent to build one…");
  ask.addEventListener("click", () => void c.invoke("oxplow.page.draft_with_agent", { text: AGENT_PROMPT }).catch(() => {}));
  intro.append(ask);

  const about = el("details", { testId: "explore-columns" });
  const lineageLine = el("p", { className: "ox-small ox-muted", testId: "explore-lineage" });
  const sql = el("textarea", {
    className: "ox-textarea",
    rows: 5,
    placeholder: "SELECT … FROM v_work_item …  or  SELECT bucket, MEASURE('…') FROM metric_grid('day')   (Cmd/Ctrl+Enter runs)",
    testId: "explore-sql",
  });
  const run = el("button", { type: "button", className: "ox-button ox-primary", testId: "explore-run" }, "Run");
  const vizSelect = el("select", { className: "ox-select", testId: "explore-viz" });
  for (const v of VIZ_OPTIONS) vizSelect.append(el("option", { value: v }, v));
  const metricSelect = el("select", { className: "ox-select", testId: "explore-metric" }, el("option", { value: "" }, "—"));
  const metricLabel = el("label", { className: "ox-small", hidden: true }, "Chart a metric ", metricSelect);
  const sliceSelect = el("select", { className: "ox-select", testId: "explore-slice" }, el("option", { value: "" }, "(none)"));
  const sliceLabel = el("label", { className: "ox-small", hidden: true }, "Slice By ", sliceSelect);
  const toolbar = el("div", { className: "ox-toolbar" }, run, el("label", { className: "ox-small" }, "Show as ", vizSelect), metricLabel, sliceLabel);
  const chartRow = el("div", { className: "ox-toolbar", testId: "explore-chart", hidden: true });
  const error = el("p", { className: "ox-error", testId: "explore-error", hidden: true });
  document.body.append(intro, about, lineageLine, sql, toolbar, chartRow, error);

  // ---- State into the page ----
  const draft = () => {
    const d = c.historyState?.sql;
    return typeof d === "string" ? d : null;
  };
  const templateActive = () => view.template !== null && sql.value === view.template.sql;
  const showError = (message: string | null, field: string | null = null) => {
    error.hidden = message === null;
    error.textContent = message ?? "";
    sql.setAttribute("aria-invalid", field === "/data/rows/sql" ? "true" : "false");
  };
  const drawControls = () => {
    if (document.activeElement !== sql) sql.value = draft() ?? view.sql;
    run.disabled = sql.value.trim() === "";
    vizSelect.value = view.viz;
    metricSelect.value = templateActive() ? view.template!.metric : "";
    sliceLabel.hidden = !templateActive();
    sliceSelect.value = view.template?.dimension ?? "";
    drawChartRoles();
  };
  const drawChartRoles = () => {
    const roles = CHART_ROLES[view.viz];
    chartRow.hidden = !roles || !view.chart || columns.length === 0;
    chartRow.replaceChildren();
    if (chartRow.hidden) return;
    for (const role of roles!) {
      const pick = el("select", { className: "ox-select", testId: `explore-chart-${role.key}` });
      if (role.optional) pick.append(el("option", { value: "" }, "(none)"));
      for (const col of columns) pick.append(el("option", { value: col }, col));
      pick.value = view.chart?.[role.key] ?? "";
      const locked = role.key === "series" && templateActive();
      pick.disabled = locked;
      if (locked) pick.title = "Slice the metric by a dimension above";
      pick.addEventListener("change", () => {
        const chart = { ...view.chart!, [role.key]: pick.value || null };
        set({ chart });
        void show();
      });
      chartRow.append(el("label", { className: "ox-small" }, `${role.label} `, pick));
    }
  };

  // The views this page wrote and hasn't heard back yet: each comes back
  // as `onViewParams`, maybe after a later write, and is no move of the
  // tab's.
  const sent: string[] = [];
  const keyOf = (v: ExploreView) => JSON.stringify(v);

  /** Change the view: the tab's address is replaced, and the page redraws. */
  const set = (patch: Partial<ExploreView>) => {
    view = { ...view, ...patch };
    const params: Record<string, string | null> = {
      model: view.model || null,
      sql: view.sql || null,
      viz: view.viz === "table" ? null : view.viz,
      chart: view.chart ? JSON.stringify(view.chart) : null,
      template: view.template ? templateParam(view.template) : null,
    };
    sent.push(keyOf(readView(Object.fromEntries(Object.entries(params).filter(([, v]) => v !== null)) as Record<string, string>)));
    void c.setViewParams(params);
    drawControls();
  };

  /** Draw the view's query below (through oxplow); a chart viz without
   *  columns that fit the result gets sensible ones, and is drawn again. */
  const show = async () => {
    if (view.sql.trim() === "") {
      columns = [];
      await c.showView(null);
      showError(null);
      drawControls();
      return;
    }
    try {
      const rows = await c.showView(specOf(view.sql, view.viz, view.chart));
      columns = rows?.columns ?? [];
      showError(null);
      if (rows && CHART_ROLES[view.viz] && !(view.chart && chartFits(view.chart, rows.columns))) {
        const chart = chartDefaults(view.viz, rows);
        if (chart) {
          set({ chart });
          await c.showView(specOf(view.sql, view.viz, chart));
        }
      }
    } catch (e) {
      const why = e as Refusal;
      showError(why.message ?? String(e), why.field ?? null);
    }
    drawControls();
  };

  const runSql = (text: string) => {
    void c.setHistoryState({ sql: null });
    set({ sql: text });
    void show();
  };
  const applyTemplate = (template: MetricTemplate) => {
    void c.setHistoryState({ sql: null });
    set({ template, viz: template.viz, chart: template.chart, sql: template.sql });
    void show();
  };

  /** The picked model's documented columns and its lineage. */
  const describe = async (model: string) => {
    about.replaceChildren();
    lineageLine.replaceChildren();
    about.hidden = model === "";
    lineageLine.hidden = model === "";
    if (model === "") return;
    const [m, cols, lin] = await Promise.all([c.querySql(MODEL_SQL, [model]), c.querySql(MODEL_COLUMNS_SQL, [model]), c.querySql(LINEAGE_SQL, [model])]);
    const description = String(rowObjects(m)[0]?.description ?? "");
    about.append(el("summary", {}, el("strong", {}, model), ` — ${description}`));
    const table = el("table", { className: "ox-table" });
    for (const col of modelColumns(cols)) {
      table.append(el("tr", {}, el("td", {}, el("code", {}, col.name)), el("td", { className: "ox-muted" }, col.sqlType), el("td", {}, col.doc)));
    }
    about.append(table);
    const { reads, readBy } = lineage(model, lin);
    const link = (name: string) => {
      const a = el("button", { type: "button", className: "ox-link ox-mono" }, name);
      a.addEventListener("click", () => pick(name));
      return a;
    };
    const join = (label: string, names: Node[]) => {
      if (names.length === 0) return;
      lineageLine.append(`${label} `);
      names.forEach((n, i) => lineageLine.append(...(i > 0 ? [", ", n] : [n])));
      lineageLine.append(". ");
    };
    join("Reads", reads.map((r) => (r.kind === "ref" ? link(r.name) : el("code", {}, r.name))));
    join("Read by", readBy.map(link));
  };

  /** A model picked: its sample query, shown as a table. */
  const pick = (model: string) => {
    void c.setHistoryState({ sql: null });
    set({ model, viz: "table", chart: null, template: null, sql: sampleSql(model) });
    void describe(model);
    void show();
  };

  // ---- What the person does ----
  sql.addEventListener("input", () => {
    void c.setHistoryState({ sql: sql.value });
    run.disabled = sql.value.trim() === "";
  });
  sql.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      runSql(sql.value);
    }
  });
  run.addEventListener("click", () => runSql(sql.value));
  vizSelect.addEventListener("change", () => {
    set({ viz: vizSelect.value as Viz, chart: null });
    void show();
  });
  metricSelect.addEventListener("change", () => {
    if (metricSelect.value) applyTemplate(metricTemplate(metricSelect.value, null));
  });
  sliceSelect.addEventListener("change", () => {
    if (view.template) applyTemplate(sliceTemplate(view.template, sliceSelect.value || null));
  });

  // The tab moved (Back, Forward) or the rail picked a model. A pick
  // changes `model` alone (the query the entry holds is the old model's);
  // a move changes the query with it.
  c.onViewParams((params) => {
    const next = readView(params);
    // An echo of a write of its own (and those before it): heard.
    const echo = sent.indexOf(keyOf(next));
    if (echo !== -1) {
      sent.splice(0, echo + 1);
      return;
    }
    if (keyOf(next) === keyOf(view)) return;
    if (next.model !== view.model && next.sql === view.sql && next.model !== "") {
      view = next;
      return pick(next.model);
    }
    const modelChanged = next.model !== view.model;
    view = next;
    if (modelChanged) void describe(view.model);
    drawControls();
    void show();
  });

  // The rail picked a model: the page's `model` param changed.
  c.onUpdate(({ params }) => {
    const next = pageModel(params);
    if (next === picked) return;
    picked = next;
    if (next !== "" && next !== view.model) pick(next);
  });

  // The metric pickers: what can be charted and sliced.
  void Promise.all([c.querySql(METRICS_SQL), c.querySql(DIMENSIONS_SQL)])
    .then(([metrics, dimensions]) => {
      for (const r of rowObjects(metrics)) metricSelect.append(el("option", { value: String(r.key) }, String(r.title ?? r.key)));
      for (const r of rowObjects(dimensions)) sliceSelect.append(el("option", { value: String(r.key) }, String(r.label ?? r.key)));
      metricLabel.hidden = metrics.rows.length === 0;
      drawControls();
    })
    .catch(() => {
      // No metrics or dimensions: the pickers stay hidden.
    });

  drawControls();
  if (picked !== "" && picked !== view.model && view.sql === "") {
    // Opened on a model the rail picked, with nothing of its own yet.
    pick(picked);
  } else {
    void describe(view.model);
    if (view.sql) void show();
  }
});
