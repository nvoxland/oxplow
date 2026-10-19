// triage/todo-board: the to-do comments as a board, a column for each kind
// the model sorted them into. A card opens its file at the comment's line,
// and has a form to decide what to do about it — keep it, fix it now, or
// won't fix — kept in the extension's private state through its own
// commands (`triage.todos.decide`, read back with `triage.todos.decisions`). If
// those commands are refused (the extension isn't deployed, or is disabled) it
// shows the TODOs without their decisions, and says so.
(function () {
  "use strict";

  const KINDS = ["bug", "cleanup", "feature"];
  const DECISIONS = [
    ["", "Undecided"],
    ["keep", "Keep"],
    ["fix-now", "Fix now"],
    ["wontfix", "Won't fix"],
  ];
  const board = document.getElementById("board");
  const status = document.getElementById("status");

  oxplow.connect().then((component) => {
    component.applyTheme();

    // The TODOs from the lens's own rows, without decisions.
    const fromRun = (run) => {
      const columns = run.result.columns;
      const at = (row, name) => row[columns.indexOf(name)];
      return run.result.rows.map((row) => ({
        path: at(row, "path"),
        line: at(row, "line"),
        kind: at(row, "kind"),
        decision: null,
      }));
    };

    // The TODOs with their decisions; the lens's rows when the board may
    // not run commands yet.
    const load = async () => {
      try {
        // A script command answers `{ result, children }`: its script's
        // own `result`, and the commands it composed (none here).
        const out = await component.invoke("triage.todos.decisions", {});
        status.textContent = "";
        return { todos: out.result.todos, decides: true };
      } catch (e) {
        status.textContent =
          e && e.code === "DENIED"
            ? "Deploy the triage extension in Settings → Extensions to record decisions."
            : "Couldn't read the decisions: " + (e && e.message ? e.message : e);
        return { todos: fromRun(component.run), decides: false };
      }
    };

    const decisionForm = (todo, decides) => {
      const form = document.createElement("form");
      form.className = "ox-form decide";
      const field = document.createElement("label");
      field.className = "ox-field";
      const label = document.createElement("span");
      label.className = "ox-label";
      label.textContent = "Decision";
      const select = document.createElement("select");
      select.className = "ox-select";
      select.name = "decision";
      select.disabled = !decides;
      for (const [value, text] of DECISIONS) {
        const option = document.createElement("option");
        option.value = value;
        option.textContent = text;
        select.appendChild(option);
      }
      // Chosen once every option is in: what was decided, or undecided.
      select.value = todo.decision || "";
      const error = document.createElement("span");
      error.className = "ox-error";
      error.setAttribute("role", "alert");
      const save = document.createElement("button");
      save.type = "submit";
      save.className = "ox-button";
      save.textContent = "Save";
      save.disabled = !decides;
      field.append(label, select);
      form.append(field, save, error);
      // A form here submits nowhere (the frame's CSP): its script runs
      // the command, and a refusal naming the field shows under it.
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        error.textContent = "";
        if (!select.value) {
          error.textContent = "Pick a decision.";
          return;
        }
        save.disabled = true;
        component
          .invoke("triage.todos.decide", { path: todo.path, line: todo.line, decision: select.value })
          .then(() => refresh())
          .catch((e) => {
            error.textContent = e && e.message ? e.message : String(e);
            save.disabled = false;
          });
      });
      return form;
    };

    const draw = ({ todos, decides }) => {
      board.textContent = "";
      for (const kind of KINDS) {
        const column = document.createElement("section");
        column.className = "column";
        const heading = document.createElement("h3");
        heading.className = "ox-heading";
        heading.textContent = kind;
        column.appendChild(heading);
        for (const todo of todos.filter((t) => t.kind === kind)) {
          const card = document.createElement("div");
          card.className = "ox-card card";
          const where = document.createElement("a");
          where.className = "ox-link";
          where.href = "#";
          where.textContent = todo.path + ":" + todo.line;
          where.addEventListener("click", (event) => {
            event.preventDefault();
            component
              .navigate("file:" + todo.path + "#L" + todo.line, { newTab: event.metaKey || event.ctrlKey })
              .catch((e) => {
                status.textContent = e.message;
              });
          });
          card.append(where, decisionForm(todo, decides));
          column.appendChild(card);
        }
        board.appendChild(column);
      }
    };

    const refresh = () => load().then(draw);
    refresh();
    component.onUpdate(() => void refresh());
  }, (e) => {
    status.textContent = "Couldn't connect to oxplow: " + (e && e.message ? e.message : e);
  });
})();
