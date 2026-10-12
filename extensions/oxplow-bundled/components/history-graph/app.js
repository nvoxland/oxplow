// oxplow-bundled/history-graph: the stream's commits as `git log --graph`
// draws them — a lane per line of descent, a dot per commit, edges to its
// parents — in place of core's plain commit list (the `vcs.history.graph`
// replacement). Its lens walks `v_commit` from the stream's head; the
// filters here only dim or hide rows, so typing never re-runs the lens.
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
    let history = read(component.run);
    let revealed = null;

    const open = (sha, newTab) => component.navigate("commit:" + sha, { newTab }).catch(say);
    const draw = () => {
      revealed = render(history, filters(), open);
      if (revealed) revealed.scrollIntoView({ block: "nearest" });
    };
    const refill = () => {
      fill(authorEl, "All authors", unique(history.commits.map((c) => c.author)));
      fill(branchEl, "All branches", unique(history.branchHeads.map((b) => b.name)));
    };

    refill();
    draw();
    // The host runs the lens again when a commit lands or a ref moves (or
    // the page asks to reveal another commit); the filters stay as set.
    component.onUpdate((run) => {
      history = read(run);
      refill();
      draw();
    });
    for (const el of [queryEl, authorEl, branchEl]) el.addEventListener("input", draw);
  }, (e) => say(new Error("Couldn't connect to oxplow: " + (e && e.message ? e.message : e))));

  /** The run's commits (children before parents), branch heads and tags,
   *  and the commit to reveal. */
  function read(run) {
    const columns = run.result.columns;
    const at = (row, name) => row[columns.indexOf(name)];
    const commits = [];
    const branchHeads = [];
    const tagsBySha = new Map();
    for (const row of run.result.rows) {
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
    const reveal = run.params && run.params.reveal_sha ? String(run.params.reveal_sha) : null;
    return { commits: topoOrder(commits), branchHeads, tagsBySha, reveal };
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

  /** Put `values` in `select` after its "all" option, keeping the choice. */
  function fill(select, all, values) {
    const chosen = select.value;
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
  function render(history, f, open) {
    const onBranch = reachable(history, f.branch);
    const commits = onBranch ? history.commits.filter((c) => onBranch.has(c.sha)) : history.commits;
    const { rows, width } = layout(commits);
    const graphWidth = PAD * 2 + LANE * Math.max(1, width);
    const x = (k) => PAD + k * LANE + LANE / 2;
    const mid = ROW / 2;
    let matched = 0;
    let revealed = null;
    rowsEl.textContent = "";
    for (const row of rows) {
      const c = row.commit;
      const hit =
        (!f.author || c.author === f.author) &&
        (!f.query ||
          c.sha.toLowerCase().includes(f.query) ||
          c.subject.toLowerCase().includes(f.query) ||
          c.author.toLowerCase().includes(f.query) ||
          c.email.toLowerCase().includes(f.query));
      if (hit) matched += 1;

      const el = document.createElement("div");
      el.className = "row" + (hit ? "" : " dimmed") + (c.sha === history.reveal ? " revealed" : "");
      el.setAttribute("role", "listitem");
      el.dataset.sha = c.sha;
      el.title = c.sha;
      el.addEventListener("click", (e) => open(c.sha, e.metaKey || e.ctrlKey || e.button === 1));

      const svg = document.createElementNS(SVG, "svg");
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
      el.appendChild(svg);

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
    countEl.textContent = matched + " / " + history.commits.length;
    return revealed;
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
