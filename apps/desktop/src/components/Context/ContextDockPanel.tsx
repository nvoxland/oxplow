/// The Context dock panel: what the person collected for agents, one tab per
/// basket. **This Thread** (the selected thread's own; its tab says which
/// baskets are attached to it) and each named basket, with its count; a
/// basket attached to this thread wears a link mark. The open tab has a bar
/// of its actions — This Thread's Clear; a named basket's Attach to This
/// Thread or Detach, Rename (in place), Clear and Delete, also on the tab's
/// right-click — over its items. An item is its name (opening its source),
/// its text as a quote (up to three lines) and the person's note, edited in
/// place (Ctrl/Cmd+Enter saves, empty removes it; "Add a note…" shows on
/// hover), led by a × to take it out; its right-click has Move to ▸ and
/// Remove. While an add waits, its card (`ContextAddCard`) is all the panel
/// shows, and the add opens the tab it went to. The open tab is remembered
/// (an app preference). The icon's badge is how many items the thread's next
/// prompt carries. The Context page is the panel header's arrow.

import { FileText, Link2, Quote, StickyNote, X } from "lucide-react";
import { useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";

import { InlineEdit } from "../InlineEdit.js";
import {
  THREAD_BASKET,
  THREAD_MARK,
  attachBasket,
  clearContext,
  deleteBasket,
  detachBasket,
  moveContextItem,
  nextPrompt,
  removeFromContext,
  renameBasket,
  setContextNote,
  useBaskets,
  usePendingAdd,
  useSelectedThreadId,
  type BasketItem,
  type Baskets,
  type Destination,
  type NamedBasket,
} from "../../contextBasket.js";
import type { MenuItem } from "../../menu.js";
import { useAppPreference } from "../../prefs/remembered.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { ContextAddCard } from "./ContextAddCard.js";
import { plainTitle, useContextTitle } from "./contextTitle.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { dockButtonStyle } from "../Dock/dockStyle.js";

/** The app preference that keeps the open tab: a basket's name, or `THREAD_MARK`. */
export const OPEN_TAB_PREF = "context.openTab";

export function useContextBadge(): DockBadge | null {
  const thread = useSelectedThreadId();
  const count = nextPrompt(useBaskets(thread), thread).length;
  return count > 0 ? { count } : null;
}

/** What a row says without reading anything: the label, else the source's
 *  name, else the text's start. */
export function itemTitle(item: BasketItem): string {
  return plainTitle({ label: item.label, uri: item.sourceUri, text: item.text });
}

/** Why a basket can't be attached to `thread` now, or null when it can. */
export function attachDisabledReason(basket: NamedBasket, thread: string | null): string | null {
  if (!thread) return "Select a thread first";
  if (basket.items.length === 0) return "Nothing in it yet";
  return null;
}

/** The tab to show: `wanted` while it is there, else This Thread (with a
 *  thread), else the first named basket. */
export function openTab(wanted: string | null, thread: string | null, names: readonly string[]): string | null {
  if (wanted === THREAD_MARK && thread) return THREAD_MARK;
  if (wanted != null && names.includes(wanted)) return wanted;
  if (thread) return THREAD_MARK;
  return names[0] ?? null;
}

export function ContextDockBody({ onOpenPage }: DockBodyProps) {
  const thread = useSelectedThreadId();
  const baskets = useBaskets(thread);
  const adding = usePendingAdd();
  const menu = useContextMenu();
  const [wanted, setWanted] = useAppPreference<string | null>(OPEN_TAB_PREF, null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const names = baskets.named.map((b) => b.name);
  const tab = openTab(wanted, thread, names);
  const named = baskets.named.find((b) => b.name === tab) ?? null;
  const attachedHere = baskets.named.filter((b) => thread != null && b.attachedTo === thread);

  const itemMenu = (e: MouseEvent, item: BasketItem) =>
    menu.open(e, [
      { id: "move", label: "Move to", enabled: true, submenu: moveTargets(baskets, thread, item) },
      { id: "remove", label: "Remove", enabled: true, run: () => removeFromContext(item.id) },
    ]);
  const rename = (name: string) => {
    setWanted(name);
    setRenaming(name);
  };
  const basketMenu = (e: MouseEvent, basket: NamedBasket) => menu.open(e, basketActions(basket, thread, () => rename(basket.name)));

  if (adding) {
    return (
      <div data-testid="context-panel" style={panelStyle}>
        <ContextAddCard add={adding} baskets={baskets} onAdded={(to: Destination) => setWanted("basket" in to ? to.basket : THREAD_MARK)} />
        {menu.menu}
      </div>
    );
  }
  if (tab == null) {
    return (
      <div data-testid="context-panel" style={panelStyle}>
        <div data-testid="context-panel-empty" style={{ color: "var(--text-muted)" }}>
          No context added yet
        </div>
      </div>
    );
  }

  const items = tab === THREAD_MARK ? baskets.own : (named?.items ?? []);
  return (
    <div data-testid="context-panel" style={panelStyle}>
      <div role="tablist" aria-label="Baskets" style={tabStrip}>
        {thread ? (
          <Tab key={THREAD_MARK} id={THREAD_MARK} on={tab === THREAD_MARK} count={baskets.own.length} onPick={setWanted}>
            {THREAD_BASKET}
          </Tab>
        ) : null}
        {baskets.named.map((b) => (
          <Tab key={b.id} id={b.name} on={tab === b.name} count={b.items.length} attached={thread != null && b.attachedTo === thread} onPick={setWanted} onContextMenu={(e) => basketMenu(e, b)}>
            {b.name}
          </Tab>
        ))}
      </div>

      <div data-testid="context-tab-bar" style={barStyle}>
        {tab === THREAD_MARK ? (
          <>
            <span style={{ color: "var(--text-muted)", flex: 1, minWidth: 0 }}>
              {attachedHere.length > 0 ? (
                <>
                  The next prompt also carries{" "}
                  {attachedHere.map((b, i) => (
                    <span key={b.id}>
                      {i > 0 ? ", " : null}
                      <button type="button" data-testid="context-attached-link" onClick={() => setWanted(b.name)} style={linkButton}>
                        {b.name}
                      </button>
                    </span>
                  ))}
                </>
              ) : (
                "The next prompt to any agent in this thread carries these."
              )}
            </span>
            {baskets.own.length > 0 && thread ? (
              <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-panel-clear" title="Take everything out of This Thread" onClick={() => void clearContext({ thread })}>
                Clear
              </button>
            ) : null}
          </>
        ) : named ? (
          renaming === named.name ? (
            <span style={{ flex: 1, minWidth: 0 }}>
              <InlineEdit
                value={named.name}
                startEditing
                ariaLabel="Basket name"
                testId="context-basket-rename"
                onCommit={(next) => {
                  setRenaming(null);
                  const name = next.trim();
                  if (name && name !== named.name) {
                    setWanted(name);
                    void renameBasket(named.name, name);
                  }
                }}
                onCancel={() => setRenaming(null)}
              />
            </span>
          ) : (
            <BasketBar basket={named} thread={thread} onRename={() => rename(named.name)} />
          )
        ) : null}
      </div>

      {items.length === 0 ? (
        <div data-testid="context-tab-empty" style={{ color: "var(--text-muted)", padding: "4px 0" }}>
          {tab === THREAD_MARK ? "Nothing yet. Add a page from its ☰, or right-click a selection." : "Nothing in this basket yet. Pick it in To when you add to context."}
        </div>
      ) : (
        <div data-testid="context-tab-items">
          {items.map((item, i) => (
            <ItemRow key={item.id} item={item} first={i === 0} onOpenPage={onOpenPage} onContextMenu={(e) => itemMenu(e, item)} />
          ))}
        </div>
      )}
      {menu.menu}
    </div>
  );
}

function Tab({
  id,
  on,
  count,
  attached = false,
  onPick,
  onContextMenu,
  children,
}: {
  id: string;
  on: boolean;
  count: number;
  attached?: boolean;
  onPick(id: string): void;
  onContextMenu?(e: MouseEvent): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={on}
      data-testid="context-tab"
      data-basket={id}
      title={attached ? "Attached: this thread's next prompt carries it" : undefined}
      onClick={() => onPick(id)}
      onContextMenu={onContextMenu}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        flexShrink: 0,
        background: "none",
        border: "none",
        borderBottom: `2px solid ${on ? "var(--accent)" : "transparent"}`,
        padding: "4px 2px 5px",
        marginBottom: -1,
        font: "inherit",
        color: on ? "var(--text-primary)" : "var(--text-secondary)",
        fontWeight: on ? 600 : 400,
        cursor: "pointer",
      }}
    >
      {attached ? <Link2 size={12} aria-label="Attached" style={{ color: "var(--accent)" }} /> : null}
      {children}
      <span data-testid="context-tab-count" style={{ color: "var(--text-muted)", fontWeight: 400 }}>
        {count}
      </span>
    </button>
  );
}

/** A named basket's actions, on its bar and its tab's right-click. */
function basketActions(basket: NamedBasket, thread: string | null, rename: () => void): MenuItem[] {
  const why = attachDisabledReason(basket, thread);
  return [
    basket.attachedTo
      ? { id: "detach", label: "Detach", enabled: true, run: () => detachBasket(basket.name) }
      : { id: "attach", label: "Attach to This Thread", enabled: why == null, hint: why ?? undefined, run: () => (thread ? attachBasket(basket.name, thread) : undefined) },
    { id: "rename", label: "Rename…", enabled: true, run: rename },
    { id: "clear", label: "Clear", enabled: basket.items.length > 0, run: () => clearContext({ basket: basket.name }) },
    { id: "delete", label: "Delete", enabled: true, run: () => deleteBasket(basket.name) },
  ];
}

function BasketBar({ basket, thread, onRename }: { basket: NamedBasket; thread: string | null; onRename(): void }) {
  const here = thread != null && basket.attachedTo === thread;
  const why = attachDisabledReason(basket, thread);
  return (
    <>
      <span data-testid="context-basket-status" style={{ color: here ? "var(--accent)" : "var(--text-muted)", flex: 1, minWidth: 0 }}>
        {here ? "Attached: this thread's next prompt carries it, then empties it." : basket.attachedTo ? "Attached to another thread." : "Not attached."}
      </span>
      {basket.attachedTo ? (
        <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-basket-detach" onClick={() => void detachBasket(basket.name)}>
          Detach
        </button>
      ) : (
        <button
          type="button"
          className="ox-button"
          style={dockButtonStyle}
          data-testid="context-basket-attach"
          disabled={why != null}
          title={why ?? "This thread's next prompt carries it, then empties it"}
          onClick={() => (thread ? void attachBasket(basket.name, thread) : undefined)}
        >
          Attach to This Thread
        </button>
      )}
      <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-basket-rename-button" onClick={onRename}>
        Rename
      </button>
      <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-basket-clear" disabled={basket.items.length === 0} onClick={() => void clearContext({ basket: basket.name })}>
        Clear
      </button>
      <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-basket-delete" onClick={() => void deleteBasket(basket.name)}>
        Delete
      </button>
    </>
  );
}

function ItemRow({ item, first, onOpenPage, onContextMenu }: { item: BasketItem; first: boolean; onOpenPage: DockBodyProps["onOpenPage"]; onContextMenu(e: MouseEvent): void }) {
  const [hover, setHover] = useState(false);
  const title = useContextTitle({ label: item.label, uri: item.sourceUri, text: item.text });
  const Icon = item.kind === "text" ? Quote : FileText;
  const showNote = item.note != null || hover;
  return (
    <div
      data-testid="context-panel-item"
      onContextMenu={onContextMenu}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onFocus={() => setHover(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setHover(false);
      }}
      style={{ display: "flex", alignItems: "flex-start", gap: 8, padding: "6px 0", borderTop: first ? "none" : "1px solid var(--border-subtle)" }}
    >
      <button type="button" data-testid="context-panel-remove" aria-label="Remove from context" title="Remove" onClick={() => void removeFromContext(item.id)} style={{ ...iconButton, marginTop: 1 }}>
        <X size={14} aria-hidden />
      </button>
      <span style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 4 }}>
        <span style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
          <Icon size={13} aria-hidden style={{ flexShrink: 0, color: "var(--text-muted)" }} />
          {item.sourceUri ? (
            <button type="button" data-testid="context-panel-title" title={item.sourceUri} onClick={() => onOpenPage({ id: item.sourceUri! })} style={{ ...linkButton, ...ellipsis, fontWeight: 600, color: "var(--text-primary)" }}>
              {title}
            </button>
          ) : (
            <span data-testid="context-panel-title" style={{ ...ellipsis, fontWeight: 600 }}>
              {title}
            </span>
          )}
        </span>
        {item.kind === "text" && item.text ? (
          <div data-testid="context-panel-quote" style={quoteStyle}>
            {item.text}
          </div>
        ) : null}
        <span style={{ display: "flex", alignItems: "flex-start", gap: 6, opacity: showNote ? 1 : 0, minHeight: 16 }}>
          <StickyNote size={12} aria-hidden style={{ flexShrink: 0, marginTop: 2, color: "var(--text-muted)" }} />
          <span style={{ flex: 1, minWidth: 0 }}>
            <InlineEdit
              value={item.note ?? ""}
              multiline
              allowEmpty
              placeholder="Add a note…"
              ariaLabel="Note"
              testId="context-panel-note"
              displayStyle={{ color: item.note ? "var(--text-primary)" : "var(--text-muted)", whiteSpace: "pre-wrap", cursor: "text" }}
              onCommit={(next) => {
                if (next.trim() !== (item.note ?? "")) void setContextNote(item.id, next);
              }}
            />
          </span>
        </span>
      </span>
    </div>
  );
}

/** Where an item can move: This Thread (with a thread) and every named
 *  basket but its own. */
export function moveTargets(baskets: Baskets, thread: string | null, item: BasketItem): MenuItem[] {
  const targets: MenuItem[] = [];
  if (thread && item.basket != null) targets.push({ id: "move-thread", label: THREAD_BASKET, enabled: true, run: () => moveContextItem(item.id, { thread }) });
  for (const b of baskets.named) {
    if (b.name !== item.basket) targets.push({ id: `move-${b.id}`, label: b.name, enabled: true, run: () => moveContextItem(item.id, { basket: b.name }) });
  }
  return targets.length > 0 ? targets : [{ id: "move-none", label: "No other basket", enabled: false }];
}

const panelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 6, padding: "6px 12px 8px", fontSize: "var(--text-xs)" };

const tabStrip: CSSProperties = {
  display: "flex",
  gap: 16,
  overflowX: "auto",
  borderBottom: "1px solid var(--border-subtle)",
  scrollbarWidth: "thin",
};

const barStyle: CSSProperties = { display: "flex", alignItems: "center", gap: 6, minHeight: 24 };

const ellipsis = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 } as const;

const quoteStyle: CSSProperties = {
  color: "var(--text-secondary)",
  borderLeft: "2px solid var(--border-subtle)",
  paddingLeft: 8,
  whiteSpace: "pre-wrap",
  fontFamily: "var(--font-mono, monospace)",
  display: "-webkit-box",
  WebkitLineClamp: 3,
  WebkitBoxOrient: "vertical",
  overflow: "hidden",
};

const linkButton: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  font: "inherit",
  color: "var(--accent)",
  cursor: "pointer",
  textAlign: "left",
};

const iconButton = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-muted)",
  cursor: "pointer",
  fontSize: "inherit",
  display: "inline-flex",
} as const;
