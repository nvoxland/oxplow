// oxplow/history-graph: the stream's commits as `git log --graph`
// draws them — a lane per line of descent, a dot per commit, edges to its
// parents — in place of core's plain commit list (the `vcs.history.graph`
// replacement). Its page walks `v_commit` from the stream's head; the
// On Git History the filters are the page's view params (`q`, `author`,
// `branch`, the replacement's contract): they start from the page's,
// each change writes them, and Back sets them again — a reload, a
// bookmark and core's own list see the same. The List shows the page's
// `history_matches`, searched in SQL over the whole history before the
// limit, so an older commit is found too; the Graph draws the loaded
// `history_graph` rows (lanes need every commit between two matches) and
// dims what doesn't match. Both also filter locally, so a change shows at
// once, before the page answers with the new search. Where its host gives
// no view (a tile) the filters are its own and local only.
// A row opens its commit; Cmd/Ctrl-click opens it in a new tab.
(function () {
  "use strict";

  const SVG = "http://www.w3.org/2000/svg";
  const ROW = 36; // px per commit
  const LANE = 14; // px per lane
  const PAD = 8; // px left and right of the lanes
  const RADIUS = 4;
  const COLORS = 8; // lane-0 … lane-7 in app.css, the theme's chart series

  const rowsEl = document.getElementById("rows");
  const queryEl = document.getElementById("query");
  const listModeEl = document.getElementById("list-mode");
  const graphModeEl = document.getElementById("graph-mode");
  const authorEl = document.getElementById("author");
  const branchEl = document.getElementById("branch");
  const countEl = document.getElementById("count");
  const emptyEl = document.getElementById("empty");
  const statusEl = document.getElementById("status");
  const say = (e) => {
    statusEl.textContent = e && e.message ? e.message : String(e);
  };

  oxplow.connect().then((component) => {
    component.applyTheme();
    let history = read(component.run, component.data);
    const savedMode = (state) => {
      if (state?.reveal === history.reveal && (state.mode === "list" || state.mode === "graph")) {
        return state.mode;
      }
      return history.reveal ? "graph" : "list";
    };
    let mode = savedMode(component.historyState);

    const open = (uri, newTab) => component.navigate(uri, { newTab }).catch(say);
    const draw = () => {
      listModeEl.setAttribute("aria-pressed", String(mode === "list"));
      graphModeEl.setAttribute("aria-pressed", String(mode === "graph"));
      const revealed = render(history, filters(), mode, open);
      if (revealed) revealed.scrollIntoView({ block: "nearest" });
    };
    const chooseMode = (next) => {
      if (mode === next) return;
      mode = next;
      draw();
      if (component.historyState) component.setHistoryState({ mode, reveal: history.reveal }).catch(say);
    };
    listModeEl.addEventListener("click", () => chooseMode("list"));
    graphModeEl.addEventListener("click", () => chooseMode("graph"));
    component.onHistoryState((state) => {
      const next = savedMode(state);
      if (mode === next) return;
      mode = next;
      draw();
    });
    const refill = (author = authorEl.value, branch = branchEl.value) => {
      fill(authorEl, "All authors", unique(history.commits.map((c) => c.author)), author);
      fill(branchEl, "All branches", unique(history.branchHeads.map((b) => b.name)), branch);
    };
    // The page's view, as the filters show it.
    const showView = (params) => {
      queryEl.value = params.q || "";
      refill(params.author || "", params.branch || "");
    };
    // Each view written and not yet heard back: its echo isn't a change
    // (typing goes on while an older write comes back).
    const written = [];
    const viewKey = (p) => JSON.stringify([p.q || "", p.author || "", p.branch || ""]);
    const writeView = () => {
      const params = { q: queryEl.value || null, author: authorEl.value || null, branch: branchEl.value || null };
      written.push(viewKey(params));
      component.setViewParams(params).catch(say);
    };

    if (component.viewParams) showView(component.viewParams);
    else refill();
    draw();
    // The host runs the page again when a commit lands or a ref moves (or
    // the page asks to reveal another commit); the filters stay as set.
    component.onUpdate(({ run, data }) => {
      const previousReveal = history.reveal;
      history = read(run, data);
      if (history.reveal !== previousReveal) mode = savedMode(component.historyState);
      refill();
      draw();
    });
    for (const el of [queryEl, authorEl, branchEl]) {
      el.addEventListener("input", () => {
        draw();
        if (component.viewParams) writeView();
      });
    }
    if (component.viewParams) {
      component.onViewParams((params) => {
        const at = written.indexOf(viewKey(params));
        if (at >= 0) {
          written.splice(0, at + 1);
          return;
        }
        written.length = 0;
        showView(params);
        draw();
      });
    }
  }, (e) => say(new Error("Couldn't connect to oxplow: " + (e && e.message ? e.message : e))));

  /** The run's commits (children before parents), branch heads and tags,
   *  the List's matches (searched in SQL, `data.history_matches`; the run's
   *  own rows where the host gives none), the filters they ran with, and
   *  the commit to reveal. */
  function read(run, data) {
    const loaded = commitsOf(run.result);
    const searched = data && data.history_matches ? commitsOf(data.history_matches) : null;
    const commits = loaded.commits;
    const reveal = run.params && run.params.reveal_sha ? String(run.params.reveal_sha) : null;
    const ranBranch = searched && run.params && run.params.branch ? String(run.params.branch) : "";
    commits.sort((a, b) => b.time - a.time || a.sha.localeCompare(b.sha));
    const listed = searched ? searched.commits : commits.slice();
    listed.sort((a, b) => b.time - a.time || a.sha.localeCompare(b.sha));
    for (const [sha, tags] of searched ? searched.tagsBySha : []) loaded.tagsBySha.set(sha, tags);
    return {
      commits,
      graphCommits: topoOrder(commits),
      listCommits: listed,
      searched: searched !== null,
      ranBranch,
      branchHeads: loaded.branchHeads,
      tagsBySha: loaded.tagsBySha,
      reveal,
    };
  }

  /** A loader's rows (`{ columns, rows }`) as commits, with the branch
   *  heads and tags they carry. */
  function commitsOf(result) {
    const columns = result.columns;
    const at = (row, name) => row[columns.indexOf(name)];
    const commits = [];
    const branchHeads = [];
    const tagsBySha = new Map();
    for (const row of result.rows) {
      const sha = String(at(row, "sha"));
      const commit = {
        sha,
        author: String(at(row, "author") || ""),
        email: String(at(row, "email") || ""),
        time: Date.parse(String(at(row, "committed_at"))),
        subject: String(at(row, "subject") || ""),
        parents: list(at(row, "parents")),
        branches: list(at(row, "branches")),
        current: at(row, "current_branch"),
      };
      commits.push(commit);
      for (const name of commit.branches) branchHeads.push({ name: String(name), sha });
      const tags = list(at(row, "tags"));
      if (tags.length > 0) tagsBySha.set(sha, tags.map(String));
    }
    return { commits, branchHeads, tagsBySha };
  }

  /** A JSON array column (`parents`, `branches`, `tags`) as a list. */
  function list(cell) {
    if (cell === null || cell === undefined || cell === "") return [];
    try {
      const parsed = JSON.parse(String(cell));
      return Array.isArray(parsed) ? parsed.filter((v) => v !== null) : [];
    } catch (_) {
      return [];
    }
  }

  function unique(values) {
    return Array.from(new Set(values)).sort((a, b) => a.localeCompare(b));
  }

  /** Put `values` in `select` after its "all" option, choosing `chosen`
   *  when it is one of them. */
  function fill(select, all, values, chosen) {
    select.textContent = "";
    const first = document.createElement("option");
    first.value = "";
    first.textContent = all;
    select.appendChild(first);
    for (const value of values) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = value;
      select.appendChild(option);
    }
    select.value = values.includes(chosen) ? chosen : "";
  }

  function filters() {
    return { query: queryEl.value.trim().toLowerCase(), author: authorEl.value, branch: branchEl.value };
  }

  /** Every child before its parents, otherwise newest first: what the
   *  lanes need. Commit times are whole seconds and clocks skew, so time
   *  order alone can put a parent before its child. */
  function topoOrder(commits) {
    const present = new Set(commits.map((c) => c.sha));
    const children = new Map();
    for (const c of commits) {
      for (const p of c.parents) if (present.has(p)) children.set(p, (children.get(p) || 0) + 1);
    }
    const bySha = new Map(commits.map((c) => [c.sha, c]));
    const rank = new Map(commits.map((c, i) => [c.sha, i]));
    const ready = commits.filter((c) => !children.get(c.sha));
    const out = [];
    while (ready.length > 0) {
      let best = 0;
      for (let i = 1; i < ready.length; i++) {
        const a = ready[i];
        const b = ready[best];
        if (a.time > b.time || (a.time === b.time && rank.get(a.sha) < rank.get(b.sha))) best = i;
      }
      const next = ready.splice(best, 1)[0];
      out.push(next);
      for (const p of next.parents) {
        const left = (children.get(p) || 0) - 1;
        children.set(p, left);
        if (left === 0 && bySha.has(p)) ready.push(bySha.get(p));
      }
    }
    return out;
  }

  /** The lanes, as `git log --graph` keeps them: each column waits for a
   *  sha; a commit takes its column over for its first parent, and extra
   *  parents open new columns. */
  function layout(commits) {
    const present = new Set(commits.map((c) => c.sha));
    let lanes = [];
    let width = 0;
    const rows = commits.map((commit) => {
      let column = lanes.indexOf(commit.sha);
      const fromAbove = column !== -1;
      if (!fromAbove) {
        column = lanes.indexOf(null);
        if (column === -1) {
          column = lanes.length;
          lanes.push(null);
        }
        lanes[column] = commit.sha;
      }
      const incoming = lanes.slice();
      for (let k = 0; k < lanes.length; k++) if (lanes[k] === commit.sha) lanes[k] = null;
      const edges = commit.parents.map((sha, i) => {
        let to = lanes.indexOf(sha);
        if (to === -1) {
          if (i === 0 && lanes[column] === null) to = column;
          else {
            to = lanes.indexOf(null);
            if (to === -1) {
              to = lanes.length;
              lanes.push(null);
            }
          }
          if (present.has(sha)) lanes[to] = sha;
        }
        return to;
      });
      while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
      const outgoing = lanes.slice();
      width = Math.max(width, incoming.length, outgoing.length);
      return { commit, column, fromAbove, incoming, outgoing, edges };
    });
    return { rows, width };
  }

  /** The shas reachable from `branch`'s head, or null for every branch. */
  function reachable(history, branch) {
    if (!branch) return null;
    const head = history.branchHeads.find((b) => b.name === branch);
    const out = new Set();
    if (!head) return out;
    const parents = new Map(history.commits.map((c) => [c.sha, c.parents]));
    const stack = [head.sha];
    while (stack.length > 0) {
      const sha = stack.pop();
      if (out.has(sha)) continue;
      out.add(sha);
      for (const p of parents.get(sha) || []) stack.push(p);
    }
    return out;
  }

  /** Draw the rows; returns the revealed row's element, if shown. */
  function render(history, f, mode, open) {
    // The List's matches were searched on the run's branch already (some
    // lie beyond the loaded rows the local walk sees).
    const searchedBranch = mode !== "graph" && history.searched && f.branch === history.ranBranch;
    const onBranch = searchedBranch ? null : reachable(history, f.branch);
    const source = mode === "graph" ? history.graphCommits : history.listCommits;
    const commits = onBranch ? source.filter((c) => onBranch.has(c.sha)) : source;
    const matches = new Set(commits.filter((c) =>
      (!f.author || c.author === f.author) &&
      (!f.query || [c.sha, c.subject, c.author, c.email].some((value) => value.toLowerCase().includes(f.query)))
    ).map((c) => c.sha));
    const { rows, width } = mode === "graph"
      ? layout(commits)
      : { rows: commits.filter((c) => matches.has(c.sha)).map((commit) => ({ commit })), width: 0 };
    let revealed = null;
    rowsEl.textContent = "";
    for (const row of rows) {
      const c = row.commit;
      const el = document.createElement("a");
      const uri = "oxplow:oxplow/commit?sha=" + encodeURIComponent(c.sha);
      el.href = uri;
      el.className = "row" + (matches.has(c.sha) ? "" : " dimmed") + (c.sha === history.reveal ? " revealed" : "");
      el.dataset.sha = c.sha;
      el.title = c.sha;
      el.addEventListener("click", (e) => {
        e.preventDefault();
        open(uri, e.metaKey || e.ctrlKey);
      });
      el.addEventListener("auxclick", (e) => {
        if (e.button !== 1) return;
        e.preventDefault();
        open(uri, true);
      });
      if (mode === "graph") el.appendChild(drawLanes(row, width));

      const subject = document.createElement("span");
      subject.className = "subject";
      subject.textContent = c.subject;
      el.appendChild(subject);
      for (const name of c.branches) {
        el.appendChild(badge(String(name), name === c.current ? "ref current" : "ref"));
      }
      for (const name of history.tagsBySha.get(c.sha) || []) el.appendChild(badge(name, "ref"));
      el.appendChild(meta("meta", c.author));
      el.appendChild(meta("meta when", stamp(c.time)));
      rowsEl.appendChild(el);
      if (c.sha === history.reveal) revealed = el;
    }
    emptyEl.hidden = rows.length > 0;
    emptyEl.textContent = history.commits.length ? "No commits match these filters." : "This branch has no history yet.";
    // Searched in SQL, the List's matches aren't a part of the loaded rows.
    countEl.textContent = mode !== "graph" && history.searched
      ? String(matches.size)
      : matches.size + " / " + history.commits.length;
    return revealed;
  }

  function drawLanes(row, width) {
    const graphWidth = PAD * 2 + LANE * Math.max(1, width);
    const x = (k) => PAD + k * LANE + LANE / 2;
    const mid = ROW / 2;
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("width", String(graphWidth));
    svg.setAttribute("height", String(ROW));
    const line = (k, x1, y1, x2, y2) => {
      const l = document.createElementNS(SVG, "line");
      l.setAttribute("x1", x1);
      l.setAttribute("y1", y1);
      l.setAttribute("x2", x2);
      l.setAttribute("y2", y2);
      l.setAttribute("stroke-width", "1.5");
      l.setAttribute("stroke-linecap", "round");
      l.setAttribute("class", "lane-" + (k % COLORS));
      svg.appendChild(l);
    };
    row.incoming.forEach((sha, k) => {
      if (!sha) return;
      if (k !== row.column) line(k, x(k), 0, x(k), ROW);
      else if (row.fromAbove) line(k, x(k), 0, x(k), mid);
    });
    if (row.outgoing[row.column]) line(row.column, x(row.column), mid, x(row.column), ROW);
    for (const to of row.edges) if (to !== row.column) line(to, x(row.column), mid, x(to), ROW);
    const dot = document.createElementNS(SVG, "circle");
    dot.setAttribute("cx", x(row.column));
    dot.setAttribute("cy", mid);
    dot.setAttribute("r", RADIUS);
    dot.setAttribute("stroke-width", "1.5");
    dot.setAttribute("class", "node-" + (row.column % COLORS) + " lane-" + (row.column % COLORS));
    svg.appendChild(dot);
    return svg;
  }

  function badge(text, cls) {
    const b = document.createElement("span");
    b.className = "ox-badge " + cls;
    b.textContent = text;
    return b;
  }

  function meta(cls, text) {
    const m = document.createElement("span");
    m.className = cls;
    m.textContent = text;
    return m;
  }

  /** `2026-04-26 14:32`, local time. */
  function stamp(ms) {
    if (Number.isNaN(ms)) return "";
    const d = new Date(ms);
    const two = (n) => String(n).padStart(2, "0");
    return d.getFullYear() + "-" + two(d.getMonth() + 1) + "-" + two(d.getDate()) + " " + two(d.getHours()) + ":" + two(d.getMinutes());
  }
})();
