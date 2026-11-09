// The Tasks page (oxplow-bundled, `pages/tasks.yaml`): the work items as a
// kanban board. A custom page: oxplow draws its nav bar (the title, New
// Task…), confirms what asks, records what fails; the frame draws the
// toolbar, the columns and a card's menu with the kit's classes. Its
// filters are the page's view params (`show`, `priority`, `q`): a change
// goes into the tab's address, the page runs again and the frame hears the
// rows; Back puts the filters back.
import { assigneesOf, boardColumns, cardHint, cardMenu, choiceFields, dropAct, epicGroups, filtersOf, moreLabel, moreRef, priorityOf, records, type Act, type Filters, type MenuContext, type MenuEntry, type Rec, type Rows } from "./model.js";

interface Component {
  data: Record<string, Rows>;
  params: Record<string, unknown>;
  viewParams: Record<string, string> | null;
  onUpdate(fn: (u: { data: Record<string, Rows>; params: Record<string, unknown> }) => void): () => void;
  invoke(command: string, input?: unknown): Promise<unknown>;
  navigate(ref: string, options?: { newTab?: boolean }): Promise<null>;
  setTitle(title: string): Promise<null>;
  setActions(actions: { id: string; label: string; primary?: boolean; prominent?: boolean; icon?: string }[]): Promise<null>;
  onAction(fn: (id: string) => void): () => void;
  setViewParams(params: Record<string, string | null>): Promise<null>;
  pageMemory: { get(key: string): Promise<unknown>; set(key: string, value: unknown): Promise<null> };
  onViewParams(fn: (params: Record<string, string>) => void): () => void;
  applyTheme(): void;
}
declare const oxplow: { connect(): Promise<Component> };

const DEFAULTS: Filters = { show: "thread", priority: "any", q: "" };
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

oxplow.connect().then(start, (e: { message?: string }) => say(`The board couldn't connect: ${e?.message ?? e}`));

function start(c: Component) {
  c.applyTheme();
  let data = c.data;
  let params = c.params;
  let view = c.viewParams;
  let dragging: Rec | null = null;
  // The epics folded in To Do: the person's, in every tab of the board.
  let folded = new Set<string>();
  c.pageMemory.get("folded-epics").then((v) => {
    if (Array.isArray(v)) folded = new Set(v.map(String));
    render();
  }, () => {});
  const fold = (uri: string) => {
    if (folded.has(uri)) folded.delete(uri);
    else folded.add(uri);
    void c.pageMemory.set("folded-epics", [...folded]).catch(() => {});
    render();
  };

  c.setTitle("Tasks");
  c.setActions([{ id: "new", label: "New Task…", prominent: true, icon: "plus" }]);
  c.onAction((id) => {
    const row = records(data.row)[0];
    if (id === "new" && row?.new_task) void c.navigate(String(row.new_task));
  });

  // The toolbar writes the view; Back and Forward write it back.
  const show = $<HTMLSelectElement>("show");
  const priority = $<HTMLSelectElement>("priority");
  const q = $<HTMLInputElement>("q");
  const filters = () => filtersOf(view, params);
  const sync = () => {
    const f = filters();
    show.value = f.show;
    priority.value = f.priority;
    if (document.activeElement !== q) q.value = f.q;
  };
  const write = (patch: Partial<Filters>) => {
    const f = { ...filters(), ...patch };
    view = { ...(view ?? {}), ...Object.fromEntries(Object.entries(f).map(([k, v]) => [k, String(v)])) };
    void c.setViewParams(Object.fromEntries(Object.entries(f).map(([k, v]) => [k, v === DEFAULTS[k as keyof Filters] ? null : v])));
  };
  show.addEventListener("change", () => write({ show: show.value }));
  priority.addEventListener("change", () => write({ priority: priority.value }));
  let typing: ReturnType<typeof setTimeout> | undefined;
  q.addEventListener("input", () => {
    clearTimeout(typing);
    typing = setTimeout(() => write({ q: q.value.trim() }), 250);
  });
  q.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      clearTimeout(typing);
      write({ q: q.value.trim() });
    } else if (e.key === "Escape" && q.value !== "") {
      e.stopPropagation();
      q.value = "";
      write({ q: "" });
    }
  });
  c.onViewParams((v) => {
    view = v;
    sync();
  });
  c.onUpdate((u) => {
    data = u.data;
    params = u.params;
    sync();
    render();
  });

  const run = (act: Act) => c.invoke(act.command, act.input).catch(() => render());

  function render() {
    closeMenu();
    const board = $<HTMLDivElement>("board");
    const failed = ["board", "counts"].map((k) => data[k]?.error).find(Boolean);
    const cards = [...records(data.board), ...records(data.live).map((r) => ({ ...r, uri: null, state: "in_progress" }))];
    const columns = boardColumns(cards, records(data.counts));
    const opts: MenuContext = { fields: choiceFields(records(data.fields)), assignees: assigneesOf(records(data.assignees)) };
    say(failed ? `The board couldn't read its items: ${failed}` : cards.length === 0 ? "Nothing here. File an item with New Task…." : null);
    board.replaceChildren(
      ...columns.map((col) => {
        const section = el("section", { class: "ox-board-column", "data-state": col.state, "data-testid": `column-${col.state}` });
        const head = el("div", { class: "column-head" });
        head.append(el("h2", { class: "ox-heading" }, col.label), el("span", { class: "ox-faint ox-small count" }, String(col.total)));
        const list = el("ul", { class: "cards" });
        if (col.state === "todo") {
          // To Do under its epics, each foldable; the loose cards last.
          for (const g of epicGroups(col.cards, cards)) {
            if (g.uri === null) list.append(...g.cards.map((card) => cardEl(card, opts)));
            else list.append(groupEl(g.uri, g.title, g.epic, g.cards, opts));
          }
        } else list.append(...col.cards.map((card) => cardEl(card, opts)));
        section.append(head, list);
        if (col.more > 0) {
          const more = el("button", { type: "button", class: "ox-link more" }, moreLabel(col));
          more.addEventListener("click", () => void c.navigate(moreRef(filters())));
          section.append(more);
        }
        dropZone(section, list, col.state);
        return section;
      }),
    );
  }

  // A compact card: its title and its description's first sentence, two
  // lines each at most; an urgent or high priority a coloured edge; the rest
  // on hover.
  function cardEl(card: Rec, opts: MenuContext, nested = false): HTMLElement {
    const live = card.uri == null;
    const prio = priorityOf(card);
    const li = el("li", {
      class: ["ox-card", live ? "live" : "ox-drag", nested ? "nested" : "", prio ? `prio-${prio}` : ""].filter(Boolean).join(" "),
      "data-uri": live ? "" : String(card.uri),
      title: cardHint(card),
    });
    const title = el(live ? "span" : "a", { class: live ? "card-title" : "card-title ox-link", ...(live ? {} : { role: "link", tabindex: "0" }) }, String(card.title ?? ""));
    li.append(title);
    const desc = card.summary == null ? null : String(card.summary);
    if (desc) li.append(el("span", { class: "card-desc" }, desc));
    if (live) return li;
    const open = () => void c.navigate(String(card.uri));
    title.addEventListener("click", open);
    title.addEventListener("keydown", (e) => {
      if (e.key === "Enter") open();
    });
    const menu = cardMenu(card, opts);
    const button = el("button", { type: "button", class: "card-menu", "aria-label": `Actions for ${card.title}`, "aria-haspopup": "menu" }, "…");
    button.addEventListener("click", (e) => {
      const r = button.getBoundingClientRect();
      e.stopPropagation();
      openMenu(menu, r.left, r.bottom);
    });
    li.append(button);
    li.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      openMenu(menu, e.clientX, e.clientY);
    });
    li.draggable = true;
    li.addEventListener("dragstart", (e) => {
      dragging = card;
      li.classList.add("ox-dragging");
      e.dataTransfer?.setData("text/plain", String(card.uri));
    });
    li.addEventListener("dragend", () => {
      dragging = null;
      li.classList.remove("ox-dragging");
      clearDrop();
    });
    return li;
  }

  // An epic in To Do: one block, a line down its left side, its heading (a
  // fold toggle, its title — a link — and how many of its items are here;
  // its own card's menu when it is in To Do) over its items.
  function groupEl(uri: string, title: string | null, epic: Rec | null, items: Rec[], opts: MenuContext): HTMLElement {
    const open = !folded.has(uri);
    const group = el("li", { class: "epic-group ox-nest", "data-epic": uri });
    const li = el("div", { class: "epic", ...(epic ? { "data-uri": uri } : {}) });
    const toggle = el("button", { type: "button", class: "epic-toggle", "aria-expanded": String(open), "aria-label": `${open ? "Fold" : "Unfold"} ${title ?? "epic"}` }, open ? "▾" : "▸");
    toggle.addEventListener("click", () => fold(uri));
    const name = el("a", { class: "epic-title ox-link", role: "link", tabindex: "0", title: title ?? "" }, title ?? "Epic");
    name.addEventListener("click", () => void c.navigate(uri));
    name.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void c.navigate(uri);
    });
    li.append(toggle, name, el("span", { class: "ox-faint ox-small" }, String(items.length)));
    if (epic) {
      const menu = cardMenu(epic, opts);
      li.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        openMenu(menu, e.clientX, e.clientY);
      });
    }
    group.append(li);
    if (open) {
      const under = el("ul", { class: "epic-cards" });
      under.append(...items.map((card) => cardEl(card, opts, true)));
      group.append(under);
    }
    return group;
  }

  function dropZone(section: HTMLElement, list: HTMLElement, state: string) {
    const beforeAt = (y: number): HTMLElement | null =>
      [...list.querySelectorAll<HTMLElement>(".ox-card:not(.live)")].find((li) => {
        const r = li.getBoundingClientRect();
        return y < r.top + r.height / 2;
      }) ?? null;
    section.addEventListener("dragover", (e) => {
      if (!dragging) return;
      e.preventDefault();
      clearDrop();
      section.classList.add("ox-drop-target");
      const before = beforeAt(e.clientY);
      if (before && dragging.state === state) before.dataset.drop = "before";
    });
    section.addEventListener("dragleave", (e) => {
      if (!section.contains(e.relatedTarget as Node | null)) section.classList.remove("ox-drop-target");
    });
    section.addEventListener("drop", (e) => {
      e.preventDefault();
      const card = dragging;
      clearDrop();
      if (!card) return;
      const beforeLi = dragging?.state === state ? beforeAt(e.clientY) : null;
      const before = beforeLi ? records(data.board).find((r) => String(r.uri) === beforeLi.dataset.uri) ?? null : null;
      const act = dropAct(card, state, before);
      if (!act) return;
      // Drawn where it lands at once; the re-run redraws it, a refusal puts it back.
      const li = document.querySelector<HTMLElement>(`.ox-card[data-uri="${CSS.escape(String(card.uri))}"]`);
      if (li) (beforeLi?.parentElement ?? list).insertBefore(li, beforeLi);
      void run(act);
    });
  }

  function clearDrop() {
    document.querySelectorAll(".ox-drop-target").forEach((n) => n.classList.remove("ox-drop-target"));
    document.querySelectorAll<HTMLElement>("[data-drop]").forEach((n) => delete n.dataset.drop);
  }

  // A card's menu: the kit's `.ox-menu` at the pointer. Escape, a click
  // outside or a scroll closes it; ↑/↓ move between its items.
  let menuEl: HTMLElement | null = null;
  function openMenu(entries: MenuEntry[], x: number, y: number) {
    closeMenu();
    if (entries.length === 0) return;
    const m = el("div", { class: "ox-menu", role: "menu", "data-testid": "card-menu" });
    for (const entry of entries) {
      if (entry.kind === "separator") m.append(el("div", { class: "ox-menu-separator", role: "separator" }));
      else if (entry.kind === "heading") m.append(el("div", { class: "ox-menu-heading" }, entry.label));
      else {
        const item = el("button", {
          type: "button", role: entry.checked === undefined ? "menuitem" : "menuitemradio",
          class: entry.danger ? "ox-menu-item ox-danger" : "ox-menu-item", "data-id": entry.id,
          ...(entry.checked === undefined ? {} : { "aria-checked": String(entry.checked) }),
        }, `${entry.checked ? "✓ " : ""}${entry.label}`);
        item.addEventListener("click", () => {
          closeMenu();
          if (entry.open) void c.navigate(entry.open);
          else if (entry.act) void run(entry.act);
        });
        m.append(item);
      }
    }
    document.body.append(m);
    const r = m.getBoundingClientRect();
    m.style.left = `${Math.max(4, Math.min(x, window.innerWidth - r.width - 4))}px`;
    m.style.top = `${Math.max(4, Math.min(y, window.innerHeight - r.height - 4))}px`;
    menuEl = m;
    m.querySelector<HTMLElement>(".ox-menu-item")?.focus();
  }
  function closeMenu() {
    menuEl?.remove();
    menuEl = null;
  }
  document.addEventListener("mousedown", (e) => {
    if (menuEl && !menuEl.contains(e.target as Node)) closeMenu();
  });
  window.addEventListener("scroll", closeMenu, true);
  document.addEventListener("keydown", (e) => {
    if (!menuEl) return;
    const items = [...menuEl.querySelectorAll<HTMLElement>(".ox-menu-item")];
    const at = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      closeMenu();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      items[(at + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
    }
  });

  sync();
  render();
}

function say(text: string | null) {
  const note = $<HTMLParagraphElement>("note");
  note.hidden = text === null;
  note.textContent = text ?? "";
}

function el(tag: string, attrs: Record<string, string> = {}, text?: string): HTMLElement {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  if (text !== undefined) n.textContent = text;
  return n;
}
