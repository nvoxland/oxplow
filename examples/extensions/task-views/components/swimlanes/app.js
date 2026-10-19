// task-views/swimlanes: the work list as one column per state. Drag a card to
// another column to move it; drag within a column to reorder it (only when the
// active list keeps an order); click a title to rename; click a card to open it.
//
// It reads `v_work_item` through its `swimlanes` lens and changes items only
// through the work-item interface commands, so it works with any work list.
(function () {
  "use strict";

  const STATES = [
    ["todo", "To do"],
    ["in_progress", "In progress"],
    ["blocked", "Blocked"],
    ["done", "Done"],
    ["canceled", "Canceled"],
  ];

  const note = document.getElementById("note");
  const board = document.getElementById("board");
  const say = (e) => {
    const text = e && e.message ? e.message : String(e);
    note.textContent =
      e && e.code === "DENIED" ? "This view can't change items: deploy the extension in Settings → Extensions. (" + text + ")" : text;
    note.hidden = false;
  };
  const clearNote = () => {
    note.textContent = "";
    note.hidden = true;
  };

  /** Whether the active work list keeps an order: its features say `ordering`. */
  function hasOrdering(run) {
    const cell = run && run.result && run.result.rows[0] ? run.result.rows[0][0] : null;
    if (cell === null || cell === undefined) return false;
    let list = cell;
    if (typeof cell === "string") {
      try {
        list = JSON.parse(cell);
      } catch (_) {
        list = cell.split(/[\s,]+/);
      }
    }
    if (Array.isArray(list)) return list.includes("ordering");
    if (list && typeof list === "object") return Object.keys(list).includes("ordering");
    return list === "ordering";
  }

  oxplow.connect().then((component) => {
    component.applyTheme();
    let current = component.run;
    let ordering = false;
    let editing = false;
    let dragRef = null;

    const reload = () =>
      component.query("swimlanes", current.params || {}).then((run) => {
        current = run;
        draw();
      }, say);
    /** Run a command; on success the lens is read again. */
    const change = (command, input) =>
      component.invoke(command, input).then(
        () => {
          clearNote();
          return reload();
        },
        (e) => {
          say(e);
          throw e;
        },
      );

    component.query("features").then(
      (run) => {
        ordering = hasOrdering(run);
      },
      () => {
        ordering = false;
      },
    );
    component.onUpdate((run) => {
      current = run;
      if (!editing) draw();
    });
    draw();

    function draw() {
      const columns = current.result.columns;
      const at = (row, name) => row[columns.indexOf(name)];
      const items = current.result.rows.map((row) => ({
        ref: at(row, "ref"),
        title: at(row, "title"),
        state: at(row, "state"),
      }));
      board.replaceChildren();
      for (const [state, label] of STATES) {
        const cards = items.filter((i) => i.state === state);
        const lane = document.createElement("section");
        lane.className = "lane";
        lane.dataset.state = state;
        const head = document.createElement("h3");
        head.textContent = label + " ";
        const count = document.createElement("span");
        count.className = "ox-badge";
        count.textContent = String(cards.length);
        head.append(count);
        lane.append(head);
        for (const item of cards) lane.append(card(item));
        lane.addEventListener("dragover", (e) => {
          e.preventDefault();
          lane.classList.add("ox-drop-target");
        });
        lane.addEventListener("dragleave", () => lane.classList.remove("ox-drop-target"));
        lane.addEventListener("drop", (e) => {
          e.preventDefault();
          lane.classList.remove("ox-drop-target");
          const ref = dragRef || (e.dataTransfer && e.dataTransfer.getData("text/plain"));
          dragRef = null;
          const from = items.find((i) => i.ref === ref);
          if (!from) return;
          if (from.state !== state) {
            change("oxplow.work_item.transition", { ref, to: state }).catch(() => {});
            return;
          }
          // Within its column: only a work list that keeps an order can be reordered.
          if (!ordering) return;
          const target = e.target.closest && e.target.closest(".card");
          const over = target ? cards.find((i) => i.ref === target.dataset.ref) : cards[cards.length - 1];
          if (!over || over.ref === ref) return;
          const input = cards.indexOf(from) > cards.indexOf(over) ? { ref, before: over.ref } : { ref, after: over.ref };
          change("oxplow.work_item.reorder", input).catch(() => {});
        });
        board.append(lane);
      }
    }

    function card(item) {
      const el = document.createElement("article");
      el.className = "card ox-card ox-drag";
      el.dataset.ref = item.ref;
      el.draggable = true;
      el.addEventListener("dragstart", (e) => {
        dragRef = item.ref;
        el.classList.add("ox-dragging");
        if (e.dataTransfer) e.dataTransfer.setData("text/plain", item.ref);
      });
      el.addEventListener("dragend", () => {
        el.classList.remove("ox-dragging");
        dragRef = null;
      });
      el.addEventListener("click", (e) => {
        if (e.target.closest(".title-form")) return;
        component.navigate(item.ref, e.metaKey || e.ctrlKey ? { newTab: true } : undefined).catch(say);
      });
      const title = document.createElement("span");
      title.className = "title";
      title.textContent = item.title;
      title.addEventListener("click", (e) => {
        e.stopPropagation();
        edit(el, title, item);
      });
      el.append(title);
      return el;
    }

    /** The title as an inline form: Enter renames, Escape leaves it as it was. */
    function edit(el, title, item) {
      if (editing) return;
      editing = true;
      const form = document.createElement("form");
      form.className = "ox-form title-form";
      const field = document.createElement("div");
      field.className = "ox-field";
      const input = document.createElement("input");
      input.className = "ox-input";
      input.value = item.title;
      input.setAttribute("aria-label", "Title");
      field.append(input);
      form.append(field);
      const error = document.createElement("div");
      error.className = "ox-error";
      error.hidden = true;
      field.append(error);
      const leave = () => {
        editing = false;
        draw();
      };
      form.addEventListener("submit", (e) => {
        e.preventDefault();
        change("oxplow.work_item.update", { ref: item.ref, title: input.value }).then(
          () => {
            editing = false;
          },
          (err) => {
            if (err && err.field) {
              error.textContent = err.message;
              error.hidden = false;
            }
          },
        );
      });
      form.addEventListener("keydown", (e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          leave();
        }
      });
      title.replaceWith(form);
      input.focus();
    }
  }, (e) => say(new Error("Couldn't connect to oxplow: " + (e && e.message ? e.message : e))));
})();
