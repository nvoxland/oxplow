import { describe, expect, test } from "bun:test";

import { fileRef, lineView, pageTabRef, wikiPageRef, withViewParams } from "./pageRefs.js";
import { fileUri, pageUri, withUriView, wikiUri } from "../uri/uri.js";
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
  modeOf,
  navigate,
  open,
  parseSession,
  dropCurrent,
  refsOf,
  reorder,
  setStrip,
  serializeSession,
  patchHistoryState,
  setHistoryState,
  slotShowing,
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
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/wiki?slug=b"]);
    expect(s.active).toBe(slotOf(s, "oxplow:oxplow/wiki?slug=b"));
    expect(s.mru[0]).toBe(slotOf(s, "oxplow:oxplow/wiki?slug=b"));
  });

  test("a page already showing in a tab activates that tab", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = open(s, e(a));
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/wiki?slug=b"]);
    expect(s.active).toBe(slotOf(s, "oxplow:oxplow/wiki?slug=a"));
  });

  test("opening a page that is showing refreshes its ref, keeping its state", () => {
    const first = { ...a, payload: { slug: "a", line: 1 } };
    const again = { ...a, payload: { slug: "a", line: 9 } };
    let s = open(emptySession(), e(first, { historyState: { scroll: 4 } }));
    s = open(s, e(again));
    expect(current(s.tabs[0]!).ref.payload).toEqual({ slug: "a", line: 9 });
    expect(current(s.tabs[0]!).historyState).toEqual({ scroll: 4 });
  });

  // A diff opened again at another line is the diff that's showing: its
  // line is its view (`#`), not what it is.
  test("a tab showing the page in another view is the one refreshed", () => {
    const at = (path: string, line: number) => withViewParams(pageTabRef("diff", { path }), lineView(line));
    let s = open(emptySession(), e(at("a.ts", 1), { historyState: { scroll: 4 } }));
    s = open(s, e(at("a.ts", 9)));
    expect(ids(s)).toEqual(["oxplow:oxplow/page?page=diff&path=a.ts#lines=9-9"]);
    expect(current(s.tabs[0]!).historyState).toEqual({ scroll: 4 });
    // Another path is another page.
    s = open(s, e(at("b.ts", 3)));
    expect(ids(s)).toHaveLength(2);
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
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=c"]);
    s = back(s, slot);
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=b"]);
    s = back(s, slot);
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=a"]);
    expect(back(s, slot)).toBe(s);
    s = forward(s, slot);
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=b"]);
  });

  test("navigate after going back drops what was forward", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = back(navigate(s, slot, e(b)), slot);
    s = navigate(s, slot, e(c));
    expect(forward(s, slot)).toBe(s);
    expect(s.tabs[0]!.entries.map((x) => x.ref.id)).toEqual(["oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/wiki?slug=c"]);
  });

  test("replace takes the current entry's place: Back skips it", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(s, slot, e(b));
    s = navigate(s, slot, e(c), { replace: true });
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=c"]);
    expect(ids(back(s, slot))).toEqual(["oxplow:oxplow/wiki?slug=a"]);
  });

  test("navigating to the page already shown changes nothing", () => {
    const s = open(emptySession(), e(a));
    expect(navigate(s, s.active!, e(a))).toBe(s);
  });

  test("navigating to a page another tab shows closes that tab", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = navigate(s, slotOf(s, "oxplow:oxplow/wiki?slug=a"), e(b));
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=b"]);
  });
});

// A page shows once per display mode: its identity plus the renderer its
// view names (`#oxplow.renderer`), absent being its own mode (the default).
describe("display modes: one tab per identity and mode", () => {
  const file = fileUri("a.ts");
  const asDefinition = withUriView(file, { "oxplow.renderer": "oxplow-foundation/definition" });
  const asBlame = withUriView(file, { "oxplow.renderer": "acme/blame" });
  const f = { id: file } as Entry["ref"];
  const def = { id: asDefinition } as Entry["ref"];
  const blame = { id: asBlame } as Entry["ref"];

  test("modeOf is the renderer the view names, or nothing", () => {
    expect(modeOf(file)).toBe("");
    expect(modeOf(asDefinition)).toBe("oxplow-foundation/definition");
    expect(modeOf(withUriView(file, { lines: "3-4" }))).toBe("");
    expect(modeOf("not a uri")).toBe("");
  });

  test("opening a file in another mode opens a second tab and leaves the first alone", () => {
    let s = open(emptySession(), e(f));
    s = open(s, e(def));
    expect(ids(s)).toEqual([file, asDefinition]);
    expect(s.active).toBe(slotOf(s, asDefinition));
    // The same mode again focuses its tab.
    s = open(s, e(f));
    expect(ids(s)).toHaveLength(2);
    expect(s.active).toBe(slotOf(s, file));
  });

  test("a tab showing the file in another view of the same mode is still the one refreshed", () => {
    let s = open(emptySession(), e(def));
    s = open(s, e({ id: withUriView(file, { "oxplow.renderer": "oxplow-foundation/definition", params: "{}" }) }));
    expect(s.tabs).toHaveLength(1);
  });

  test("slotShowing tells modes apart", () => {
    const s = open(open(emptySession(), e(f)), e(def));
    expect(slotShowing(s, file)).toBe(slotOf(s, file));
    expect(slotShowing(s, asDefinition)).toBe(slotOf(s, asDefinition));
    expect(slotShowing(s, asBlame)).toBeNull();
  });

  test("navigating a tab to the file in another mode leaves the other tab open", () => {
    let s = open(open(emptySession(), e(f)), e(a));
    s = navigate(s, slotOf(s, "oxplow:oxplow/wiki?slug=a"), e(def));
    expect(ids(s)).toEqual([file, asDefinition]);
  });

  test("navigating a tab to the file in a mode another tab shows closes that tab", () => {
    let s = open(open(emptySession(), e(def)), e(a));
    s = navigate(s, slotOf(s, "oxplow:oxplow/wiki?slug=a"), e(def));
    expect(ids(s)).toEqual([asDefinition]);
  });

  test("View as: the tab switches mode in place and a tab already in that mode closes", () => {
    let s = open(open(open(emptySession(), e(f)), e(def)), e(a));
    const slot = slotOf(s, file);
    s = navigate(s, slot, e(def), { replace: true });
    expect(ids(s)).toEqual([asDefinition, "oxplow:oxplow/wiki?slug=a"]);
    expect(s.tabs[0]!.slot).toBe(slot);
    // Another mode with no tab of its own: in place, nothing else closes.
    s = navigate(s, slot, e(blame), { replace: true });
    expect(ids(s)).toEqual([asBlame, "oxplow:oxplow/wiki?slug=a"]);
  });

  test("saved state carries across a filter within a mode, not across a change of mode", () => {
    let s = open(emptySession(), e(f, { historyState: { scroll: 7 }, from: { items: [], index: 0, title: "t" } }));
    const slot = s.active!;
    s = navigate(s, slot, e({ id: withUriView(file, { lines: "3-4" }) }), { replace: true });
    expect(current(s.tabs[0]!).historyState).toEqual({ scroll: 7 });
    expect(current(s.tabs[0]!).from?.title).toBe("t");
    s = navigate(s, slot, e(def), { replace: true });
    expect(current(s.tabs[0]!).historyState).toBeUndefined();
    expect(current(s.tabs[0]!).from).toBeUndefined();
  });

  test("a body is one per identity and mode", () => {
    let s = open(emptySession(), e(f));
    const slot = s.active!;
    s = navigate(s, slot, e(def));
    expect(bodies(s.tabs[0]!).map((b) => b.key)).toEqual([`${slot}/${file}`, `${slot}/${file}\noxplow-foundation/definition`]);
  });
});

describe("navigate in a page's own view", () => {
  test("a filter replaces the entry in place, keeping its state, its origin and both directions", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(s, slot, e(pageTabRef("x/y"), { historyState: { scroll: 40 } }));
    s = navigate(s, slot, e(c));
    s = back(s, slot);
    s = navigate(s, slot, e(pageTabRef("x/y", {}, { range: "7d" })), { replace: true });
    const tab = s.tabs[0]!;
    expect(current(tab).ref.id).toBe("oxplow:oxplow/page?page=x/y#range=7d");
    expect(current(tab).historyState).toEqual({ scroll: 40 });
    expect(tab.entries.map((x) => x.ref.id)).toEqual(["oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/page?page=x/y#range=7d", "oxplow:oxplow/wiki?slug=c"]);
  });

  test("a selection's entry starts with the page's state, so open sections stay open", () => {
    let s = open(emptySession(), e(pageTabRef("x/y"), { historyState: { collapsed: ["files"] } }));
    s = navigate(s, s.active!, e(pageTabRef("x/y", {}, { commit: "abc" })));
    expect(current(s.tabs[0]!).historyState).toEqual({ collapsed: ["files"] });
  });

  test("a selection pushes: Back returns to the page without it", () => {
    let s = open(emptySession(), e(pageTabRef("x/y")));
    const slot = s.active!;
    s = navigate(s, slot, e(pageTabRef("x/y", {}, { commit: "abc" })));
    expect(ids(s)).toEqual(["oxplow:oxplow/page?page=x/y#commit=abc"]);
    expect(ids(back(s, slot))).toEqual(["oxplow:oxplow/page?page=x/y"]);
  });

  test("another resource of the same page is another page, starting afresh", () => {
    let s = open(emptySession(), e(pageTabRef("x/y", { stream_id: 1 }), { historyState: { scroll: 9 } }));
    s = navigate(s, s.active!, e(pageTabRef("x/y", { stream_id: 2 })));
    expect(current(s.tabs[0]!).historyState).toBeUndefined();
    expect(bodies(s.tabs[0]!)).toHaveLength(2);
  });
});

test("setHistoryState keeps an entry's memory with it", () => {
  let s = open(emptySession(), e(a));
  const slot = s.active!;
  s = setHistoryState(s, slot, { scroll: 120 });
  s = back(navigate(s, slot, e(b)), slot);
  expect(current(s.tabs[0]!).historyState).toEqual({ scroll: 120 });
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
    expect(ids(back(s, slot))).toEqual(["oxplow:oxplow/wiki?slug=a"]);
  });

  test("a live origin steps through the items it is given", () => {
    const live: Origin = { title: "L", items: [], index: 0, live: { node: "x/y", params: {}, column: "k" } };
    let s = open(emptySession(), e(fileRef("src/a.ts"), { from: live }));
    s = stepOrigin(s, s.active!, 1, [{ ref: fileRef("src/a.ts"), label: "a" }, { ref: fileRef("src/b.ts"), label: "b" }]);
    const now = current(s.tabs[0]!);
    expect(now.ref.id).toBe(fileRef("src/b.ts").id);
    expect(now.from?.live?.node).toBe("x/y");
  });

  test("a step out of range changes nothing", () => {
    const s = open(emptySession(), e(fileRef("src/a.ts"), { from: origin }));
    expect(stepOrigin(s, s.active!, 3)).toBe(s);
    expect(stepOrigin(s, s.active!, -1)).toBe(s);
  });
});

describe("bodies: what stays mounted", () => {
  // Every page is a function of its entry: one body per page, whatever its
  // view params, so a filter or a pick re-renders it and never remounts it.
  test("entries of one page share one body, which shows the current one", () => {
    let s = open(emptySession(), e(pageTabRef("x/y")));
    const slot = s.active!;
    s = navigate(s, slot, e(pageTabRef("x/y", {}, { commit: "abc" })));
    s = navigate(s, slot, e(a));
    const out = bodies(s.tabs[0]!);
    expect(out.map((x) => x.key)).toEqual([`${slot}/${pageUri("x/y")}`, `${slot}/${wikiUri("a")}`]);
    // The page body shows its nearest entry: the one with the commit.
    expect(out[0]!.entry.ref.id).toBe("oxplow:oxplow/page?page=x/y#commit=abc");
    expect(out.map((x) => x.current)).toEqual([false, true]);
    s = back(s, slot);
    expect(bodies(s.tabs[0]!)[0]!).toMatchObject({ current: true, entry: { ref: { id: "oxplow:oxplow/page?page=x/y#commit=abc" } } });
  });

  test("a built-in page is one body across its view too", () => {
    let s = open(emptySession(), e(pageTabRef("settings", {}, { area: "agents" })));
    const slot = s.active!;
    s = navigate(s, slot, e(pageTabRef("settings", {}, { area: "data" })));
    expect(bodies(s.tabs[0]!)).toMatchObject([{ key: `${slot}/${pageUri("settings")}`, current: true, entry: { ref: { id: "oxplow:oxplow/page?page=settings#area=data" } } }]);
  });

  test("identityOf is the URI without its view", () => {
    expect(identityOf("oxplow:oxplow/page?page=x/y&stream_id=2#q=a&range=7d")).toBe("oxplow:oxplow/page?page=x/y&stream_id=2");
    expect(identityOf("not a uri")).toBe("not a uri");
  });
});

describe("closing, ordering, eviction", () => {
  test("closing the active tab activates the most recent other one, as a browser does; closing another keeps it", () => {
    let s = open(open(open(emptySession(), e(a)), e(b)), e(c));
    const [sa, sb, sc] = [slotOf(s, "oxplow:oxplow/wiki?slug=a"), slotOf(s, "oxplow:oxplow/wiki?slug=b"), slotOf(s, "oxplow:oxplow/wiki?slug=c")];
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
    expect(ids(reorder(s, [slotOf(s, "oxplow:oxplow/wiki?slug=c"), slotOf(s, "oxplow:oxplow/wiki?slug=a")]))).toEqual(["oxplow:oxplow/wiki?slug=c", "oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/wiki?slug=b"]);
  });

  test("activate makes a tab most recent", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = activate(s, slotOf(s, "oxplow:oxplow/wiki?slug=a"));
    expect(s.mru[0]).toBe(slotOf(s, "oxplow:oxplow/wiki?slug=a"));
  });

  test("evictions are the least recent beyond the cap, never the active, pinned or protected", () => {
    let s = emptySession();
    for (const n of ["p", "q", "r", "t", "u"]) s = open(s, e(wikiPageRef(n)));
    s = { ...s, tabs: s.tabs.map((t) => (current(t).ref.id === "oxplow:oxplow/wiki?slug=p" ? { ...t, pinned: true as const } : t)) };
    const out = evictions(s, 2, (t) => current(t).ref.id === "oxplow:oxplow/wiki?slug=q");
    expect(out.map((slot) => current(s.tabs.find((t) => t.slot === slot)!).ref.id).sort()).toEqual(["oxplow:oxplow/wiki?slug=r", "oxplow:oxplow/wiki?slug=t"]);
  });
});

describe("persistence", () => {
  test("a session round-trips, with its history, its active tab and its JSON state", () => {
    let s = open(emptySession(), e(a, { historyState: { scroll: 3 } }));
    s = navigate(s, s.active!, e(pageTabRef("x/y", { range: "7d" })));
    const back2 = parseSession(serializeSession(s));
    expect(back2).toEqual(s);
  });

  test("state that isn't JSON is dropped, not the entry", () => {
    const s = open(emptySession(), e(a, { historyState: { at: 1, node: { f: () => 1 } } }));
    const out = parseSession(serializeSession(s))!;
    expect(current(out.tabs[0]!).ref.id).toBe("oxplow:oxplow/wiki?slug=a");
    expect(current(out.tabs[0]!).historyState).toBeUndefined();
  });

  test("a stored session that isn't one reads as nothing", () => {
    expect(parseSession("nope")).toBeNull();
    expect(parseSession(JSON.stringify({ tabs: 3 }))).toBeNull();
  });
});

describe("setStrip: the strip as a list of pages", () => {
  test("keeps the tabs still listed with their history, opens the new ones and drops the rest, in the list's order", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    const sa = slotOf(s, "oxplow:oxplow/wiki?slug=a");
    s = navigate(s, sa, e(c));
    s = setStrip(s, [wikiPageRef("d"), c]);
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=d", "oxplow:oxplow/wiki?slug=c"]);
    expect(s.tabs[1]!.slot).toBe(sa);
    expect(s.tabs[1]!.entries).toHaveLength(2);
  });

  test("a pinned ref opens pinned; the same list is the same session", () => {
    const pinned = { ...a, pinned: true as const };
    const s = setStrip(emptySession(), [pinned, b]);
    expect(s.tabs[0]!.pinned).toBe(true);
    expect(setStrip(s, refsOf(s))).toBe(s);
    expect(refsOf(s).map((r) => r.id)).toEqual(["oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/wiki?slug=b"]);
  });

  test("dropping the active tab activates the most recent one left", () => {
    let s = open(open(emptySession(), e(a)), e(b));
    s = setStrip(s, [a]);
    expect(s.active).toBe(slotOf(s, "oxplow:oxplow/wiki?slug=a"));
  });
});

describe("dropCurrent: the page's record is gone", () => {
  test("goes back, dropping the entry, so Forward never leads to it", () => {
    let s = open(emptySession(), e(a));
    const slot = s.active!;
    s = navigate(navigate(s, slot, e(b)), slot, e(c));
    s = back(s, slot);
    s = dropCurrent(s, slot);
    expect(ids(s)).toEqual(["oxplow:oxplow/wiki?slug=a"]);
    expect(s.tabs[0]!.entries.map((x) => x.ref.id)).toEqual(["oxplow:oxplow/wiki?slug=a", "oxplow:oxplow/wiki?slug=c"]);
  });

  test("with nothing to go back to, closes the tab", () => {
    const s = open(emptySession(), e(a));
    expect(dropCurrent(s, s.active!).tabs).toEqual([]);
  });
});

test("patchHistoryState merges keys into the entry's state and ignores an unchanged write", () => {
  let s = open(emptySession(), e(wikiPageRef("a")));
  const slot = s.tabs[0]!.slot;
  s = patchHistoryState(s, slot, { scrollY: 40 });
  s = patchHistoryState(s, slot, { collapsed: ["x"] });
  expect(current(s.tabs[0]!).historyState).toEqual({ scrollY: 40, collapsed: ["x"] });
  expect(patchHistoryState(s, slot, { scrollY: 40, collapsed: ["x"] })).toBe(s);
});
