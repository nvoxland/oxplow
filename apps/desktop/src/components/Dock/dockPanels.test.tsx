import { expect, test } from "bun:test";

import golden from "./corePanels.golden.json";
import { CORE_ORDER, orderDockPanels, type DockPanelDef } from "./dockPanels.js";

const def = (id: string, title = id): DockPanelDef => ({ id, title, icon: "box", useBadge: () => null, Body: () => null });

test("core panels come first in the fixed order, extensions after by title", () => {
  const ordered = orderDockPanels(
    [def("alerts"), def("assistant"), def("context")],
    [def("ext:b", "Zed"), def("ext:a", "Alpha")],
  );
  expect(ordered.map((p) => p.id)).toEqual(["alerts", "assistant", "context", "ext:a", "ext:b"]);
});

test("the core dock panels only shrink", () => {
  expect(CORE_ORDER.filter((id) => !golden.includes(id))).toEqual([]); // a new panel is an extension's
  expect(golden.filter((id) => !CORE_ORDER.includes(id))).toEqual([]); // one that left: take it out of the golden too
});
