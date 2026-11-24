// oxplow/history-graph: Git History's commits while no filter is set, as
// `git log --graph` draws them: a lane per line of descent, a dot per
// commit, edges to its parents, the branches and tags at each. Its page
// walks `v_commit` from the stream's head (the `graph` loader); with a
// filter set the page shows its own table of the matches instead, so this
// draws no filters of its own. The commit to reveal (`reveal_sha`) is
// marked and scrolled to. A row opens its commit; Cmd/Ctrl-click or a
// middle click opens it in a new tab. Its rows look like oxplow's tables:
// the kit's tokens, a header row, a rule under each row.
(function () {
  "use strict";

  const SVG = "http://www.w3.org/2000/svg";
  const ROW = 36; // px per commit
  const LANE = 14; // px per lane
  const PAD = 8; // px left and right of the lanes
  const RADIUS = 4;
  const COLORS = 8; // lane-0 … lane-7 in app.css, the theme's chart series

  const rowsEl = document.getElementById("rows");
  const emptyEl = document.getElementById("empty");
  const statusEl = document.getElementById("status");
  const say = (e) => {
    statusEl.textContent = e && e.message ? e.message : String(e);
  };

  oxplow.connect().then((component) => {
    component.applyTheme();
    const open = (uri, newTab) => component.navigate(uri, { newTab }).catch(say);
    const draw = (run) => {
      const revealed = render(read(run), open);
      if (revealed) revealed.scrollIntoView({ block: "nearest" });
    };
    draw(component.run);
    // The host runs the page again when a commit lands or a ref moves (or
    // the page asks to reveal another commit).
    component.onUpdate(({ run }) => draw(run));
  }, (e) => say(new Error("Couldn't connect to oxplow: " + (e && e.message ? e.message : e))));

  /** The run's commits, every child before its parents, and the commit to
   *  reveal. */
  function read(run) {
    const { commits, tagsBySha } = commitsOf(run.result);
    commits.sort((a, b) => b.time - a.time || a.sha.localeCompare(b.sha));
    const reveal = run.params && run.params.reveal_sha ? String(run.params.reveal_sha) : null;
    return { commits: topoOrder(commits), tagsBySha, reveal };
  }

  /** A loader's rows (`{ columns, rows }`) as commits, with the tags
   *  they carry. */
  function commitsOf(result) {
    const columns = result.columns;
    const at = (row, name) => row[columns.indexOf(name)];
    const commits = [];
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
      const tags = list(at(row, "tags"));
      if (tags.length > 0) tagsBySha.set(sha, tags.map(String));
    }
    return { commits, tagsBySha };
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

  /** Draw the rows; returns the revealed row's element, if shown. */
  function render(history, open) {
    const { rows, width } = layout(history.commits);
    let revealed = null;
    rowsEl.textContent = "";
    for (const row of rows) {
      const c = row.commit;
      const el = document.createElement("a");
      const uri = "oxplow:oxplow/commit?sha=" + encodeURIComponent(c.sha);
      el.href = uri;
      el.className = "row" + (c.sha === history.reveal ? " revealed" : "");
      el.dataset.sha = c.sha;
      el.addEventListener("click", (e) => {
        e.preventDefault();
        open(uri, e.metaKey || e.ctrlKey);
      });
      el.addEventListener("auxclick", (e) => {
        if (e.button !== 1) return;
        e.preventDefault();
        open(uri, true);
      });
      el.appendChild(drawLanes(row, width));
      const subject = document.createElement("span");
      subject.className = "subject";
      subject.textContent = c.subject;
      subject.title = c.sha.slice(0, 7) + " " + c.subject;
      el.appendChild(subject);
      for (const name of c.branches) {
        el.appendChild(badge(String(name), name === c.current ? "ref current" : "ref"));
      }
      for (const name of history.tagsBySha.get(c.sha) || []) el.appendChild(badge(name, "ref"));
      el.appendChild(meta("meta author", c.author));
      const when = meta("meta when", relative(c.time));
      when.title = stamp(c.time);
      el.appendChild(when);
      rowsEl.appendChild(el);
      if (c.sha === history.reveal) revealed = el;
    }
    emptyEl.hidden = rows.length > 0;
    emptyEl.textContent = "This branch has no history yet.";
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

  /** How long ago, as oxplow's tables say it: `just now`, `5 min ago`,
   *  `3 h ago`, `2 d ago`, then the date. */
  function relative(ms) {
    if (Number.isNaN(ms)) return "";
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 60) return "just now";
    if (s < 3600) return Math.floor(s / 60) + " min ago";
    if (s < 86400) return Math.floor(s / 3600) + " h ago";
    if (s < 7 * 86400) return Math.floor(s / 86400) + " d ago";
    return stamp(ms).slice(0, 10);
  }

  /** `2026-04-26 14:32`, local time. */
  function stamp(ms) {
    if (Number.isNaN(ms)) return "";
    const d = new Date(ms);
    const two = (n) => String(n).padStart(2, "0");
    return d.getFullYear() + "-" + two(d.getMonth() + 1) + "-" + two(d.getDate()) + " " + two(d.getHours()) + ":" + two(d.getMinutes());
  }
})();
