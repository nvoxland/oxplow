import { describe, expect, test } from "bun:test";

import { fileRef, lensRef, pageTabRef, wikiPageRef } from "./pageRefs.js";
import {
  activate,
  back,
  bodies,
  close,
  current,
  emptySession,
  evictions,
  forward,
  identityOf,
  navigate,
  open,
  parseSession,
  reorder,
  replaceRef,
  serializeSession,
  setState,
  stepOrigin,
  type Entry,
  type Origin,
  type Session,
} from "./session.js";

const e = (ref: Entry["ref"], more: Partial<Entry> = {}): Entry => ({ ref, ...more });
const a = wikiPageRef("a");
const b = wikiPageRef("b");
const c = wikiPageRef("c");
const ids = (s: Session) => s.tabs.map((t) => current(t).ref.id);
const slotOf = (s: Session, id: string) => s.tabs.find((t) => current(t).ref.id === id)!.slot;

describe("open", () => {
  test("a new page is a new tab, made active and most recent", () => {
    let s = open(emptySession(), e(a));
    s = open(s, e(b));
    expect(ids(s)).toEqual(["wiki:a", "wiki:b"]);
    expect(s.active).toBe(slotOf(s, "wiki:b"));
    expect(s.mru[0]).toBe(slotOf(s, "wiki:b"));
  });

  test("a page already showing in a tab activates that tab", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = open(s, e(a));
    expect(ids(s)).toEqual(["wiki:a", "wiki:b"]);
    expect(s.active).toBe(slotOf(s, "wiki:a"));
  });

  test("slots are stable names, never ids", () => {
    const s = open(open(emptySession(), e(a)), e(b));
    expect(new Set(s.tabs.map((t) => t.slot)).size).toBe(2);
    expect(s.tabs.every((t) => !t.slot.includes(":"))).toBe(true);
  });
});

describe("history", () => {
  test("navigate pushes; back and forward move along the entries", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(s, slot, e(b));
    s = navigate(s, slot, e(c));
    expect(ids(s)).toEqual(["wiki:c"]);
    s = back(s, slot);
    expect(ids(s)).toEqual(["wiki:b"]);
    s = back(s, slot);
    expect(ids(s)).toEqual(["wiki:a"]);
    expect(back(s, slot)).toBe(s);
    s = forward(s, slot);
    expect(ids(s)).toEqual(["wiki:b"]);
  });

  test("navigate after going back drops what was forward", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = back(navigate(s, slot, e(b)), slot);
    s = navigate(s, slot, e(c));
    expect(forward(s, slot)).toBe(s);
    expect(s.tabs[0]!.entries.map((x) => x.ref.id)).toEqual(["wiki:a", "wiki:c"]);
  });

  test("replace takes the current entry's place: Back skips it", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(s, slot, e(b));
    s = navigate(s, slot, e(c), { replace: true });
    expect(ids(s)).toEqual(["wiki:c"]);
    expect(ids(back(s, slot))).toEqual(["wiki:a"]);
  });

  test("navigating to the page already shown changes nothing", () => {
    const s = open(emptySession(), e(a));
    expect(navigate(s, s.active!, e(a))).toBe(s);
  });

  test("navigating to a page another tab shows closes that tab", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = navigate(s, slotOf(s, "wiki:a"), e(b));
    expect(ids(s)).toEqual(["wiki:b"]);
  });
});

describe("replaceRef: a page changes its own view", () => {
  test("a filter replaces the entry, keeping its state, its origin and both directions", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(s, slot, e(lensRef("x/y"), { state: { scroll: 40 } }));
    s = navigate(s, slot, e(c));
    s = back(s, slot);
    s = replaceRef(s, slot, lensRef("x/y", { range: "7d" }));
    const tab = s.tabs[0]!;
    expect(current(tab).ref.id).toBe("lens:x/y?range=7d");
    expect(current(tab).state).toEqual({ scroll: 40 });
    expect(tab.entries.map((x) => x.ref.id)).toEqual(["wiki:a", "lens:x/y?range=7d", "wiki:c"]);
  });

  test("a selection pushes: Back returns to the page without it", () => {
    let s = open(emptySession(), e(lensRef("x/y")));
    const slot = s.active!;
    s = replaceRef(s, slot, lensRef("x/y", { commit: "abc" }), { push: true });
    expect(ids(s)).toEqual(["lens:x/y?commit=abc"]);
    expect(ids(back(s, slot))).toEqual(["lens:x/y"]);
  });
});

test("setState keeps an entry's memory with it", () => {
  let s = open(emptySession(), e(a));
  const slot = s.active!;
  s = setState(s, slot, { scroll: 120 });
  s = back(navigate(s, slot, e(b)), slot);
  expect(current(s.tabs[0]!).state).toEqual({ scroll: 120 });
});

describe("origin: the collection a page was opened from", () => {
  const origin: Origin = {
    title: "Files in src",
    items: [
      { ref: fileRef("src/a.ts"), label: "a.ts" },
      { ref: fileRef("src/b.ts"), label: "b.ts" },
      { ref: fileRef("src/c.ts"), label: "c.ts" },
    ],
    index: 0,
  };

  test("stepping moves through it in place, history untouched", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(s, slot, e(fileRef("src/a.ts"), { from: origin }));
    s = stepOrigin(s, slot, 2);
    const now = current(s.tabs[0]!);
    expect(now.ref.id).toBe(fileRef("src/c.ts").id);
    expect(now.from?.index).toBe(2);
    expect(s.tabs[0]!.entries).toHaveLength(2);
    expect(ids(back(s, slot))).toEqual(["wiki:a"]);
  });

  test("a step out of range changes nothing", () => {
    const s = open(emptySession(), e(fileRef("src/a.ts"), { from: origin }));
    expect(stepOrigin(s, s.active!, 3)).toBe(s);
    expect(stepOrigin(s, s.active!, -1)).toBe(s);
  });
});

describe("bodies: what stays mounted", () => {
  const live = (ref: Entry["ref"]) => ref.kind === "lens";

  test("entries of one page whose view is live share one body, which shows the current one", () => {
    let s = open(emptySession(), e(lensRef("x/y")));
    const slot = s.active!;
    s = replaceRef(s, slot, lensRef("x/y", { commit: "abc" }), { push: true });
    s = navigate(s, slot, e(a));
    const out = bodies(s.tabs[0]!, live);
    expect(out.map((x) => x.key)).toEqual([`${slot}/lens:x/y`, `${slot}/wiki:a`]);
    // The lens body shows its nearest entry: the one with the commit.
    expect(out[0]!.entry.ref.id).toBe("lens:x/y?commit=abc");
    expect(out.map((x) => x.current)).toEqual([false, true]);
    s = back(s, slot);
    expect(bodies(s.tabs[0]!, live)[0]!).toMatchObject({ current: true, entry: { ref: { id: "lens:x/y?commit=abc" } } });
  });

  test("a page whose view isn't live gets a body per entry, as before", () => {
    let s = open(emptySession(), e(pageTabRef("settings", { area: "agents" })));
    const slot = s.active!;
    s = navigate(s, slot, e(pageTabRef("settings", { area: "data" })));
    expect(bodies(s.tabs[0]!, live).map((x) => x.key)).toEqual([`${slot}/page:settings?area=agents`, `${slot}/page:settings?area=data`]);
  });

  test("identityOf is the ref without its view", () => {
    expect(identityOf("lens:x/y?range=7d&q=a")).toBe("lens:x/y");
    expect(identityOf("file:src/a.ts@HEAD")).toBe("file:src/a.ts@HEAD");
  });
});

describe("closing, ordering, eviction", () => {
  test("closing the active tab activates the most recent other one, as a browser does; closing another keeps it", () => {
    let s = open(open(open(emptySession(), e(a)), e(b)), e(c));
    const [sa, sb, sc] = [slotOf(s, "wiki:a"), slotOf(s, "wiki:b"), slotOf(s, "wiki:c")];
    expect(close(s, sa).active).toBe(sc);
    s = activate(s, sa);
    s = close(s, sa);
    expect(s.active).toBe(sc);
    expect(s.mru).not.toContain(sa);
    s = close(close(s, sb), sc);
    expect(s.active).toBeNull();
  });

  test("reorder puts tabs in the given order; slots it doesn't name keep their place after", () => {
    const s = open(open(open(emptySession(), e(a)), e(b)), e(c));
    expect(ids(reorder(s, [slotOf(s, "wiki:c"), slotOf(s, "wiki:a")]))).toEqual(["wiki:c", "wiki:a", "wiki:b"]);
  });

  test("activate makes a tab most recent", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = activate(s, slotOf(s, "wiki:a"));
    expect(s.mru[0]).toBe(slotOf(s, "wiki:a"));
  });

  test("evictions are the least recent beyond the cap, never the active, pinned or protected", () => {
    let s = emptySession();
    for (const n of ["p", "q", "r", "t", "u"]) s = open(s, e(wikiPageRef(n)));
    s = { ...s, tabs: s.tabs.map((t) => (current(t).ref.id === "wiki:p" ? { ...t, pinned: true as const } : t)) };
    const out = evictions(s, 2, (t) => current(t).ref.id === "wiki:q");
    expect(out.map((slot) => current(s.tabs.find((t) => t.slot === slot)!).ref.id).sort()).toEqual(["wiki:r", "wiki:t"]);
  });
});

describe("persistence", () => {
  test("a session round-trips, with its history, its active tab and its JSON state", () => {
    let s = open(emptySession(), e(a, { state: { scroll: 3 } }));
    s = navigate(s, s.active!, e(lensRef("x/y", { range: "7d" })));
    const back2 = parseSession(serializeSession(s));
    expect(back2).toEqual(s);
  });

  test("state that isn't JSON is dropped, not the entry", () => {
    const s = open(emptySession(), e(a, { state: { at: 1, node: { f: () => 1 } } }));
    const out = parseSession(serializeSession(s))!;
    expect(current(out.tabs[0]!).ref.id).toBe("wiki:a");
    expect(current(out.tabs[0]!).state).toBeUndefined();
  });

  test("a stored session that isn't one reads as nothing", () => {
    expect(parseSession("nope")).toBeNull();
    expect(parseSession(JSON.stringify({ tabs: 3 }))).toBeNull();
  });
});
