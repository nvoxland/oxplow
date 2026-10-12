// triage/todo-board: the to-do comments as a board, a column for each kind
// the model sorted them into. A card opens its file at the comment's line.
(function () {
  "use strict";

  const KINDS = ["bug", "cleanup", "feature"];
  const board = document.getElementById("board");
  const status = document.getElementById("status");

  oxplow.connect().then((component) => {
    component.applyTheme();
    const draw = (run) => {
      const columns = run.result.columns;
      const at = (row, name) => row[columns.indexOf(name)];
      board.textContent = "";
      for (const kind of KINDS) {
        const column = document.createElement("section");
        column.className = "column";
        const heading = document.createElement("h3");
        heading.className = "ox-heading";
        heading.textContent = kind;
        column.appendChild(heading);
        for (const row of run.result.rows.filter((r) => at(r, "kind") === kind)) {
          const card = document.createElement("div");
          card.className = "ox-card card";
          card.textContent = at(row, "path") + ":" + at(row, "line");
          card.addEventListener("click", () =>
            component.navigate("file:" + at(row, "path") + "#L" + at(row, "line")).catch((e) => {
              status.textContent = e.message;
            }),
          );
          column.appendChild(card);
        }
        board.appendChild(column);
      }
    };
    draw(component.run);
    component.onUpdate(draw);
  }, (e) => {
    status.textContent = "Couldn't connect to oxplow: " + (e && e.message ? e.message : e);
  });
})();
