import { expect, test } from "bun:test";

import { orderDockPanels, type DockPanelDef } from "./dockPanels.js";

const def = (id: string, title = id): DockPanelDef => ({ id, title, icon: "box", useBadge: () => null, Body: () => null });

test("core panels come first in the fixed order, extensions after by title", () => {
  const ordered = orderDockPanels(
    [def("alerts"), def("assistant"), def("context")],
    [def("ext:b", "Zed"), def("ext:a", "Alpha")],
  );
  expect(ordered.map((p) => p.id)).toEqual(["assistant", "context", "alerts", "ext:a", "ext:b"]);
});
