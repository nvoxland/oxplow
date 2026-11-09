// task_views/swimlanes: the work list as one column per stage of the active
// list, in its order. Drag a card to another column to move it; drag within a
// column to reorder it (only when the active list keeps an order); click a
// title to rename; click a card to open it.
//
// It reads `v_work_item` through its page's `swimlanes` loader and the list's
// stages through its `stages` loader, and changes items only through the
// work-item interface commands, so it works with any work list.
(function () {
  "use strict";

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

  oxplow.connect().then((component) => {
    component.applyTheme();
    let current = component.data.swimlanes;
    let stageRows = component.data.stages;
    let editing = false;
    let dragRef = null;
    // A card is being dragged: redrawing would remove it from under the
    // pointer and the browser would drop the drag. An update that arrives
    // meanwhile is drawn when the drag ends.
    let dragging = false;
    let stale = false;

    /** Run a command; on success the host runs the page again and sends the
     *  new rows to `onUpdate`. */
    const change = (command, input) =>
      component.invoke(command, input).then(
        () => clearNote(),
        (e) => {
          say(e);
          throw e;
        },
      );

    component.onUpdate(({ data }) => {
      current = data.swimlanes;
      stageRows = data.stages;
      if (editing || dragging) stale = true;
      else draw();
    });
    draw();

    function draw() {
      const read = (data) => {
        const at = (row, name) => row[data.columns.indexOf(name)];
        return (names) => data.rows.map((row) => Object.fromEntries(names.map((n) => [n, at(row, n)])));
      };
      const items = read(current)(["uri", "title", "stage"]);
      // One lane per stage of the active list, in its order.
      const stages = read(stageRows)(["value", "label"]);
      board.replaceChildren();
      for (const { value: stage, label } of stages) {
        const cards = items.filter((i) => i.stage === stage);
        const lane = document.createElement("section");
        lane.className = "lane";
        lane.dataset.stage = stage;
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
          const from = items.find((i) => i.uri === ref);
          if (!from) return;
          if (from.stage !== stage) {
            change("oxplow.work_item.transition", { uri: ref, to: stage }).catch(() => {});
            return;
          }
          // Within its column: its place in its assignee's queue.
          const target = e.target.closest && e.target.closest(".card");
          const over = target ? cards.find((i) => i.uri === target.dataset.ref) : cards[cards.length - 1];
          if (!over || over.uri === ref) return;
          const input = cards.indexOf(from) > cards.indexOf(over) ? { uri: ref, before: over.uri } : { uri: ref, after: over.uri };
          change("oxplow.work_item.reorder", input).catch(() => {});
        });
        board.append(lane);
      }
    }

    function card(item) {
      const el = document.createElement("article");
      el.className = "card ox-card ox-drag";
      el.dataset.ref = item.uri;
      el.draggable = true;
      el.addEventListener("dragstart", (e) => {
        dragRef = item.uri;
        dragging = true;
        el.classList.add("ox-dragging");
        if (e.dataTransfer) e.dataTransfer.setData("text/plain", item.uri);
      });
      el.addEventListener("dragend", () => {
        el.classList.remove("ox-dragging");
        dragRef = null;
        dragging = false;
        if (stale && !editing) {
          stale = false;
          draw();
        }
      });
      el.addEventListener("click", (e) => {
        if (e.target.closest(".title-form")) return;
        component.navigate(item.uri, e.metaKey || e.ctrlKey ? { newTab: true } : undefined).catch(say);
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
        change("oxplow.work_item.update", { uri: item.uri, title: input.value }).then(
          () => leave(),
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
