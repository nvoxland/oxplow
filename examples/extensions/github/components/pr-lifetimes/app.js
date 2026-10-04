// github/pr-lifetimes: each pull request as a bar from when it was opened to
// when it was merged — or to now, while it's open. The kit's charts plot
// points and series, not ranges, which is why this is a component.
//
// It re-runs its own lens with another `state` (the filter buttons), opens a
// pull request's page when its bar is clicked, and runs the `prs` sync —
// which oxplow refuses until you approve this component on Programs.
(function () {
  "use strict";

  const SVG = "http://www.w3.org/2000/svg";
  const ROW = 24; // px per pull request
  const LABEL = 260; // px for "#12 title"
  const DAY = 24 * 60 * 60 * 1000;

  oxplow.connect().then((component) => {
    component.applyTheme();
    const status = document.getElementById("status");
    const say = (e) => {
      status.textContent = e && e.message ? e.message : String(e);
    };
    let state = "all";
    const show = (run) => draw(run, (ref) => component.navigate(ref).catch(say));
    const refresh = () => component.query("pr-lifetimes", { state }).then(show, say);

    show(component.run);
    component.onUpdate(show);
    for (const button of document.querySelectorAll("[data-state]")) {
      button.addEventListener("click", () => {
        state = button.dataset.state;
        for (const other of document.querySelectorAll("[data-state]")) {
          other.classList.toggle("ox-primary", other === button);
        }
        refresh();
      });
    }
    document.getElementById("sync").addEventListener("click", () => {
      status.textContent = "Syncing…";
      component.invoke("collector.sync", { owner: "github", id: "prs" }).then(() => {
        status.textContent = "Synced.";
        refresh();
      }, say);
    });
  });

  /** The run's pull requests as bars on one time axis. */
  function draw(run, open) {
    const columns = run.result.columns;
    const at = (row, name) => row[columns.indexOf(name)];
    const now = Date.now();
    const prs = run.result.rows.map((row) => {
      const opened = Date.parse(at(row, "opened_at"));
      const merged = at(row, "merged_at") ? Date.parse(at(row, "merged_at")) : null;
      return { number: at(row, "number"), title: at(row, "title"), opened, merged, end: merged || now };
    });
    const svg = document.getElementById("chart");
    svg.replaceChildren();
    document.getElementById("empty").hidden = prs.length > 0;
    if (prs.length === 0) {
      svg.setAttribute("height", "0");
      return;
    }
    const width = Math.max(svg.clientWidth || 640, LABEL + 120);
    const from = Math.min(...prs.map((p) => p.opened));
    const to = Math.max(...prs.map((p) => p.end));
    const span = Math.max(to - from, DAY);
    const x = (t) => LABEL + ((t - from) / span) * (width - LABEL - 8);
    svg.setAttribute("height", String(prs.length * ROW + 4));
    svg.setAttribute("viewBox", `0 0 ${width} ${prs.length * ROW + 4}`);
    prs.forEach((pr, i) => {
      const ref = `github_pr:${pr.number}`;
      const bar = el("g", { class: "bar", role: "listitem", tabindex: "0", "data-ref": ref });
      const days = Math.max(1, Math.round((pr.end - pr.opened) / DAY));
      const title = el("title");
      title.textContent = `#${pr.number} ${pr.title} — ${pr.merged ? `merged after ${days} d` : `open for ${days} d`}`;
      bar.append(title);
      const label = el("text", { x: "0", y: String(i * ROW + 16) });
      label.textContent = `#${pr.number} ${pr.title}`.slice(0, 40);
      bar.append(label);
      bar.append(
        el("rect", {
          class: pr.merged ? "ox-series-3" : "ox-series-2",
          x: String(x(pr.opened)),
          y: String(i * ROW + 4),
          width: String(Math.max(3, x(pr.end) - x(pr.opened))),
          height: String(ROW - 8),
          rx: "3",
        }),
      );
      bar.addEventListener("click", () => open(ref));
      bar.addEventListener("keydown", (e) => {
        if (e.key === "Enter") open(ref);
      });
      svg.append(bar);
    });
  }

  function el(name, attrs) {
    const node = document.createElementNS(SVG, name);
    for (const key of Object.keys(attrs || {})) node.setAttribute(key, attrs[key]);
    return node;
  }
})();
