import { describe, expect, test } from "bun:test";
import { offerForShortcut } from "./keybindings.js";
import type { CommandEntry } from "./components/quickOpenResults.js";

const offer = (id: string, shortcut: string, whileTyping = false, enabled = true): CommandEntry => ({
  id,
  group: "G",
  label: id,
  searchKey: id,
  shortcut,
  whileTyping,
  enabled,
  run: () => {},
});

const offers = [
  offer("oxplow.editor.save", "Ctrl/Cmd+S", true),
  offer("oxplow.window.quick_open", "Ctrl/Cmd+P", true),
  offer("oxplow.work_item.create", "Ctrl/Cmd+Shift+N"),
];

describe("offerForShortcut", () => {
  test("a key runs the offer whose `ui.shortcut` it is, Ctrl or Cmd alike", () => {
    expect(offerForShortcut(offers, eventLike("s", { metaKey: true }), false)?.id).toBe("oxplow.editor.save");
    expect(offerForShortcut(offers, eventLike("p", { ctrlKey: true }), false)?.id).toBe("oxplow.window.quick_open");
    expect(offerForShortcut(offers, eventLike("N", { metaKey: true, shiftKey: true }), false)?.id).toBe(
      "oxplow.work_item.create",
    );
  });

  test("typing keeps a shortcut unless it runs while typing; other keys run nothing", () => {
    expect(offerForShortcut(offers, eventLike("N", { metaKey: true, shiftKey: true }), true)).toBeNull();
    expect(offerForShortcut(offers, eventLike("s", { metaKey: true }), true)?.id).toBe("oxplow.editor.save");
    expect(offerForShortcut(offers, eventLike("s"), false)).toBeNull();
    expect(offerForShortcut(offers, eventLike("s", { altKey: true, metaKey: true }), false)).toBeNull();
    expect(offerForShortcut(offers, eventLike("n", { metaKey: true }), false)).toBeNull();
  });

  test("of the offers on one key, the one whose `when` holds now runs — oxplow's own before an extension's", () => {
    const shared = [
      offer("acme.notes.save", "Ctrl/Cmd+S", true, true),
      offer("oxplow.editor.save", "Ctrl/Cmd+S", true, true),
    ];
    const s = eventLike("s", { metaKey: true });
    expect(offerForShortcut(shared, s, false)?.id).toBe("oxplow.editor.save");
    shared[1] = offer("oxplow.editor.save", "Ctrl/Cmd+S", true, false);
    expect(offerForShortcut(shared, s, false)?.id).toBe("acme.notes.save");
    shared[0] = offer("acme.notes.save", "Ctrl/Cmd+S", true, false);
    expect(offerForShortcut(shared, s, false)).toBeNull();
  });
});

function eventLike(
  key: string,
  overrides: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {},
) {
  return { key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...overrides };
}
