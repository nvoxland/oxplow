import { describe, expect, test } from "bun:test";
import { buildTabMenu } from "./tabMenu.js";
import type { WindowTab } from "../../window/windowTab.js";

const tab = (id: string, extra: Partial<WindowTab> = {}): WindowTab => ({ id, closable: true, ...extra }) as WindowTab;
const closable = (...ids: string[]) => ids.map((id) => ({ id, closable: true }));

describe("buildTabMenu", () => {
  test("no close handler: only the tab's own entries", () => {
    const own = { id: "x", label: "X", enabled: true, run: () => {} };
    const menu = buildTabMenu({ tab: tab("b", { contextMenu: [own] }), bulkClosable: closable("a", "b"), activeId: "b", onActivate: () => {} });
    expect(menu).toEqual([own]);
  });

  test("middle tab: all three items enabled, no separator when nothing else", () => {
    const menu = buildTabMenu({ tab: tab("b"), bulkClosable: closable("a", "b", "c"), activeId: "b", onClose: () => {}, onActivate: () => {} }) as any[];
    expect(menu.map((m) => [m.id, m.enabled])).toEqual([
      ["tab.close", true],
      ["tab.close-others", true],
      ["tab.close-right", true],
    ]);
  });

  test("last tab: Close Tabs to the Right is disabled", () => {
    const menu = buildTabMenu({ tab: tab("c"), bulkClosable: closable("a", "b", "c"), activeId: "c", onClose: () => {}, onActivate: () => {} }) as any[];
    expect(menu.find((m) => m.id === "tab.close-right").enabled).toBe(false);
    expect(menu.find((m) => m.id === "tab.close-others").enabled).toBe(true);
  });

  test("only tab: both bulk items disabled", () => {
    const menu = buildTabMenu({ tab: tab("a"), bulkClosable: closable("a"), activeId: "a", onClose: () => {}, onActivate: () => {} }) as any[];
    expect(menu.find((m) => m.id === "tab.close-others").enabled).toBe(false);
    expect(menu.find((m) => m.id === "tab.close-right").enabled).toBe(false);
  });

  test("pinned tab has no Close item", () => {
    const menu = buildTabMenu({ tab: tab("a", { pinned: true }), bulkClosable: [{ id: "a", closable: false }, ...closable("b")], activeId: "a", onClose: () => {}, onActivate: () => {} }) as any[];
    expect(menu.find((m) => m.id === "tab.close")).toBeUndefined();
    expect(menu.find((m) => m.id === "tab.close-others").enabled).toBe(true);
  });

  test("own entries get a separator before the close items", () => {
    const own = { id: "x", label: "X", enabled: true, run: () => {} };
    const menu = buildTabMenu({ tab: tab("a", { contextMenu: [own] }), bulkClosable: closable("a"), activeId: "a", onClose: () => {}, onActivate: () => {} }) as any[];
    expect(menu.map((m) => m.id)).toEqual(["x", "tab.close-sep", "tab.close", "tab.close-others", "tab.close-right"]);
  });

  test("menuAtOpen promise entries are included", async () => {
    const late = { id: "late", label: "L", enabled: true, run: () => {} };
    const menu = await buildTabMenu({ tab: tab("a", { menuAtOpen: async () => [late] }), bulkClosable: closable("a"), activeId: "a", onClose: () => {}, onActivate: () => {} });
    expect(menu[0].id).toBe("late");
  });

  test("Close Others closes the rest and re-activates the anchor when the active tab goes", () => {
    const closed: string[] = []; const activated: string[] = [];
    const menu = buildTabMenu({ tab: tab("b"), bulkClosable: closable("a", "b", "c"), activeId: "a", onClose: (id) => closed.push(id), onActivate: (id) => activated.push(id) }) as any[];
    menu.find((m) => m.id === "tab.close-others").run();
    expect(closed).toEqual(["a", "c"]);
    expect(activated).toEqual(["b"]);
  });
});
