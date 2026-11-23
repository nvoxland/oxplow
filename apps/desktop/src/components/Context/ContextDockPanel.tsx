/// The Context dock panel: what the person collected for agents, one tab per
/// basket, drawn as the main area's tabs and nav bar are. **This Thread**
/// (the selected thread's own) and each named basket, with its count; the
/// one tab the thread's next prompt sends wears a paper plane (`sendingNext`).
/// Under the tabs, a bar in the nav bar's colors holds the open tab's actions
/// on the left: the **Send Next** switch (the same paper plane, icon only, its
/// tooltip saying what happens; on for the basket that goes), then This
/// Thread's Clear, or a named basket's Rename (in place), Clear and Delete —
/// also on the tab's right-click; at the bar's right, what the thread's last
/// send carried and **Put Back**. What Put Back returns to This Thread is
/// held ("held, will not send" on its row): This Thread's Send Next is then
/// off, and pressing it releases the held items to go with the next prompt. A send carries one basket and then sets the
/// thread back to This Thread. Its items are one row each: the name, a note
/// mark (white when it has one, grey when not), its text on one line (cut to fit) and a ×
/// at the far right. A row's click expands it in place (animated): its whole
/// text, its note edited in place, and Open / Move to…; another
/// click (or Escape) folds it. A row's right-click has Move to ▸ and Remove. While an add waits, its card
/// (`ContextAddCard`) is all the panel shows, and the add opens the tab it
/// went to. The open tab is remembered (an app preference). The icon's badge
/// is how many items the thread's next prompt carries. The Context page is
/// the panel header's arrow.

import { ChevronRight, FileText, Quote, Send, StickyNote, X } from "lucide-react";
import { useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";

import { InlineEdit } from "../InlineEdit.js";
import {
  THREAD_BASKET,
  THREAD_MARK,
  clearContext,
  deleteBasket,
  moveContextItem,
  nextPrompt,
  putBack,
  removeFromContext,
  renameBasket,
  sendNext,
  sendingNext,
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
import { BarButton, barButtonStyle, dockToolbarStyle } from "../Dock/DockToolbar.js";
import { ContextAddCard } from "./ContextAddCard.js";
import { plainTitle, useContextTitle } from "./contextTitle.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";

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

/** Why a basket can't be sent next by `thread` now, or null when it can. */
export function sendDisabledReason(basket: NamedBasket, thread: string | null): string | null {
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
  const [expanded, setExpanded] = useState<number | null>(null);
  const names = baskets.named.map((b) => b.name);
  const tab = openTab(wanted, thread, names);
  const named = baskets.named.find((b) => b.name === tab) ?? null;
  const sending = sendingNext(baskets, thread);
  const pick = (id: string) => {
    setExpanded(null);
    setWanted(id);
  };
  const rename = (name: string) => {
    pick(name);
    setRenaming(name);
  };

  const itemActions = (item: BasketItem): MenuItem[] => [
    { id: "move", label: "Move to", enabled: true, submenu: moveTargets(baskets, thread, item) },
    { id: "remove", label: "Remove", enabled: true, run: () => removeFromContext(item.id) },
  ];
  const basketMenu = (e: MouseEvent, basket: NamedBasket) => menu.open(e, basketActions(basket, thread, () => rename(basket.name)));

  if (adding) {
    return (
      <div data-testid="context-panel" style={{ ...bodyStyle, padding: "8px 12px" }}>
        <ContextAddCard add={adding} baskets={baskets} onAdded={(to: Destination) => pick("basket" in to ? to.basket : THREAD_MARK)} />
        {menu.menu}
      </div>
    );
  }
  if (tab == null) {
    return (
      <div data-testid="context-panel" style={{ ...bodyStyle, padding: "8px 12px" }}>
        <div data-testid="context-panel-empty" style={{ color: "var(--text-muted)" }}>
          No context added yet
        </div>
      </div>
    );
  }

  const items = tab === THREAD_MARK ? baskets.own : (named?.items ?? []);
  const held = baskets.own.filter((i) => i.held).length;
  return (
    <div data-testid="context-panel" style={bodyStyle}>
      <div role="tablist" aria-label="Baskets" style={tabStrip}>
        {thread ? (
          <Tab key={THREAD_MARK} id={THREAD_MARK} on={tab === THREAD_MARK} count={baskets.own.length} sending={sending === THREAD_MARK} onPick={pick}>
            {THREAD_BASKET}
          </Tab>
        ) : null}
        {baskets.named.map((b) => (
          <Tab key={b.id} id={b.name} on={tab === b.name} count={b.items.length} sending={sending === b.name} onPick={pick} onContextMenu={(e) => basketMenu(e, b)}>
            {b.name}
          </Tab>
        ))}
      </div>

      <div data-testid="context-tab-bar" style={dockToolbarStyle}>
        {tab === THREAD_MARK ? (
          <>
            <SendToggle
              on={sending === THREAD_MARK && held === 0}
              disabledReason={thread ? null : "Select a thread first"}
              tip={
                held > 0
                  ? `Send Next: ${held === 1 ? "1 item put back is" : `${held} items put back are`} held. Press to send ${held === 1 ? "it" : "them"} with this thread's next prompt.`
                  : sending === THREAD_MARK
                    ? "Sending next: this thread's next prompt carries This Thread, then empties it. Send Next on another basket to send it instead."
                    : "Send Next: this thread's next prompt carries This Thread instead of the basket chosen now."
              }
              onPress={() => (thread && (sending !== THREAD_MARK || held > 0) ? void sendNext(thread, null) : undefined)}
            />
            <BarButton data-testid="context-panel-clear" disabled={baskets.own.length === 0 || !thread} title="Take everything out of This Thread" onClick={() => thread && void clearContext({ thread })}>
              Clear
            </BarButton>
          </>
        ) : named && renaming === named.name ? (
          <span style={{ flex: 1, minWidth: 0, maxWidth: 320 }}>
            <InlineEdit
              value={named.name}
              startEditing
              ariaLabel="Basket name"
              testId="context-basket-rename"
              onCommit={(next) => {
                setRenaming(null);
                const name = next.trim();
                if (name && name !== named.name) {
                  pick(name);
                  void renameBasket(named.name, name);
                }
              }}
              onCancel={() => setRenaming(null)}
            />
          </span>
        ) : named ? (
          <BasketBar basket={named} thread={thread} onRename={() => rename(named.name)} />
        ) : null}
        {baskets.lastSend && thread ? (
          <span data-testid="context-last-send" style={{ marginLeft: "auto", display: "inline-flex", alignItems: "center", gap: 4, color: "var(--text-muted)" }}>
            Sent {baskets.lastSend.items} {baskets.lastSend.items === 1 ? "item" : "items"} from {baskets.lastSend.basket ?? THREAD_BASKET}
            <BarButton data-testid="context-put-back" title="Put them back in their basket, as they were before the send" onClick={() => void putBack(thread)}>
              Put Back
            </BarButton>
          </span>
        ) : null}
      </div>

      <div style={{ padding: "0 12px 8px" }}>
        {items.length === 0 ? (
          <div data-testid="context-tab-empty" style={{ color: "var(--text-muted)", padding: "8px 0" }}>
            {tab === THREAD_MARK ? "Nothing yet. Add a page from its ☰, or right-click a selection." : "Nothing in this basket yet. Pick it in To when you add to context."}
          </div>
        ) : (
          <div data-testid="context-tab-items" role="list">
            {items.map((i) => (
              <ItemRow
                key={i.id}
                item={i}
                expanded={expanded === i.id}
                onToggle={() => setExpanded((now) => (now === i.id ? null : i.id))}
                onOpenPage={onOpenPage}
                onMove={(e) => menu.open(e, moveTargets(baskets, thread, i))}
                onContextMenu={(e) => menu.open(e, itemActions(i))}
              />
            ))}
          </div>
        )}
      </div>
      {menu.menu}
    </div>
  );
}

function Tab({
  id,
  on,
  count,
  sending = false,
  onPick,
  onContextMenu,
  children,
}: {
  id: string;
  on: boolean;
  count: number;
  sending?: boolean;
  onPick(id: string): void;
  onContextMenu?(e: MouseEvent): void;
  children: ReactNode;
}) {
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      role="tab"
      aria-selected={on}
      data-testid="context-tab"
      data-basket={id}
      title={sending ? "This thread's next prompt sends this basket" : undefined}
      onClick={() => onPick(id)}
      onContextMenu={onContextMenu}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        flexShrink: 0,
        padding: "7px 14px 6px",
        background: on ? "var(--accent-soft-bg)" : hover ? "var(--button-secondary-bg-hover)" : "transparent",
        color: on ? "var(--accent)" : "var(--text-secondary)",
        border: "none",
        borderRight: "1px solid var(--border-strong)",
        borderBottom: on ? "3px solid var(--accent)" : "3px solid transparent",
        font: "inherit",
        fontSize: "var(--text-sm)",
        fontWeight: on ? 600 : 400,
        cursor: "pointer",
      }}
    >
      {children}
      <span data-testid="context-tab-count" style={{ color: "var(--text-muted)", fontWeight: 400 }}>
        {count}
      </span>
      {sending ? <Send size={10} aria-label="Sends next" data-testid="context-tab-sending" style={{ color: "var(--text-muted)" }} /> : null}
    </button>
  );
}

/** A named basket's actions, on its tab's right-click. */
function basketActions(basket: NamedBasket, thread: string | null, rename: () => void): MenuItem[] {
  const why = sendDisabledReason(basket, thread);
  const here = thread != null && basket.nextFor === thread;
  return [
    here
      ? { id: "send-own", label: "Send This Thread Instead", enabled: true, run: () => sendNext(thread, null) }
      : { id: "send-next", label: "Send Next", enabled: why == null, hint: why ?? undefined, run: () => (thread ? sendNext(thread, basket.name) : undefined) },
    { id: "rename", label: "Rename…", enabled: true, run: rename },
    { id: "clear", label: "Clear", enabled: basket.items.length > 0, run: () => clearContext({ basket: basket.name }) },
    { id: "delete", label: "Delete", enabled: true, run: () => deleteBasket(basket.name) },
  ];
}

/** The Send Next switch: the tab marker's paper plane, icon only; its
 *  tooltip says what pressing it does. On for the basket that goes. */
function SendToggle({ on, disabledReason, tip, onPress }: { on: boolean; disabledReason: string | null; tip: string; onPress(): void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label="Send Next"
      data-testid="context-send-next"
      disabled={disabledReason != null}
      title={disabledReason ?? tip}
      onClick={onPress}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        ...barButtonStyle(hover && disabledReason == null),
        minWidth: 26,
        justifyContent: "center",
        // On, it wears the accent, as the tab that goes does.
        ...(on ? { background: "var(--accent-soft-bg)", color: "var(--accent)", boxShadow: "inset 0 0 0 1px var(--accent)" } : {}),
      }}
    >
      <Send size={13} aria-hidden />
    </button>
  );
}

/** A named basket's bar: Send Next, then Rename, Clear and Delete. On, the
 *  switch is this thread's choice; pressing it sends This Thread again. Sent
 *  next by another thread, it says so, and pressing it moves it here. */
function BasketBar({ basket, thread, onRename }: { basket: NamedBasket; thread: string | null; onRename(): void }) {
  const here = thread != null && basket.nextFor === thread;
  const why = here ? null : sendDisabledReason(basket, thread);
  return (
    <>
      <SendToggle
        on={here}
        disabledReason={why}
        tip={
          here
            ? "Sending next: this thread's next prompt carries this basket instead of This Thread, then empties it. Click to send This Thread instead."
            : "Send Next: this thread's next prompt carries this basket instead of This Thread, then empties it."
        }
        onPress={() => (thread ? void sendNext(thread, here ? null : basket.name) : undefined)}
      />
      <BarButton data-testid="context-basket-rename-button" onClick={onRename}>
        Rename
      </BarButton>
      <BarButton data-testid="context-basket-clear" disabled={basket.items.length === 0} onClick={() => void clearContext({ basket: basket.name })}>
        Clear
      </BarButton>
      <BarButton data-testid="context-basket-delete" onClick={() => void deleteBasket(basket.name)}>
        Delete
      </BarButton>
      {basket.nextFor && !here ? (
        <span data-testid="context-basket-status" style={{ color: "var(--text-muted)", marginLeft: 6 }}>
          Sent next by another thread
        </span>
      ) : null}
    </>
  );
}

/** One item: a row — a chevron, its name, a note mark, its text on one line
 *  and a × at the far right — that expands in place (the details grow open,
 *  `grid-template-rows` 0fr → 1fr) to its whole text, its note edited in
 *  place, and Open / Move to…. Its click or Enter toggles it; Escape
 *  (outside the note's field) folds it. Folded, the details are `inert`. */
function ItemRow({
  item,
  expanded,
  onToggle,
  onOpenPage,
  onMove,
  onContextMenu,
}: {
  item: BasketItem;
  expanded: boolean;
  onToggle(): void;
  onOpenPage: DockBodyProps["onOpenPage"];
  onMove(e: MouseEvent): void;
  onContextMenu(e: MouseEvent): void;
}) {
  const [hover, setHover] = useState(false);
  const title = useContextTitle({ label: item.label, uri: item.sourceUri, text: item.text });
  const Icon = item.kind === "text" ? Quote : FileText;
  const keys = (e: KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onToggle();
    }
  };
  return (
    <div
      role="listitem"
      onKeyDown={(e) => {
        const field = e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLInputElement;
        if (e.key === "Escape" && expanded && !field) {
          e.preventDefault();
          e.stopPropagation();
          onToggle();
        }
      }}
      style={{ margin: "0 -6px", borderBottom: "1px solid var(--border-subtle)", background: expanded ? "var(--button-secondary-bg-hover)" : "transparent" }}
    >
      <div
        tabIndex={0}
        aria-expanded={expanded}
        data-testid="context-panel-item"
        onClick={onToggle}
        onKeyDown={keys}
        onContextMenu={onContextMenu}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "5px 6px",
          background: hover ? "var(--button-secondary-bg-hover)" : "transparent",
          cursor: "pointer",
          minWidth: 0,
        }}
      >
        <ChevronRight
          size={12}
          aria-hidden
          style={{ flexShrink: 0, color: "var(--text-muted)", transform: expanded ? "rotate(90deg)" : "none", transition: "transform 150ms ease" }}
        />
        <Icon size={13} aria-hidden style={{ flexShrink: 0, color: "var(--text-muted)" }} />
        <span data-testid="context-panel-title" style={{ ...ellipsis, flex: "0 1 30%", fontWeight: 600 }}>
          {title}
        </span>
        <span
          data-testid="context-panel-note-mark"
          aria-label={item.note ? `Note: ${item.note}` : "No note"}
          title={item.note ?? "Open to add a note"}
          style={{ display: "inline-flex", flexShrink: 0, color: item.note ? "var(--text-primary)" : "var(--text-muted)" }}
        >
          <StickyNote size={13} aria-hidden />
        </span>
        {item.held ? (
          <span
            data-testid="context-panel-held"
            title="Put back: the next prompt doesn't carry it. Press Send Next on This Thread to send it."
            style={{ flexShrink: 0, color: "var(--text-muted)", fontStyle: "italic" }}
          >
            held, will not send
          </span>
        ) : null}
        <span
          data-testid="context-panel-preview"
          style={{ ...ellipsis, flex: "1 1 0", color: "var(--text-secondary)", fontFamily: "var(--font-mono, monospace)" }}
        >
          {item.text ? item.text.replace(/\s+/g, " ").trim() : ""}
        </span>
        <button
          type="button"
          data-testid="context-panel-remove"
          aria-label="Remove from context"
          title="Remove"
          onClick={(e) => {
            e.stopPropagation();
            void removeFromContext(item.id);
          }}
          style={iconButton}
        >
          <X size={14} aria-hidden />
        </button>
      </div>
      <div style={{ display: "grid", gridTemplateRows: expanded ? "1fr" : "0fr", transition: "grid-template-rows 180ms ease" }}>
        <div data-testid="context-item-details" inert={!expanded} aria-hidden={!expanded} style={{ overflow: "hidden", minHeight: 0 }}>
          <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: "2px 6px 10px 26px" }}>
            {item.text ? (
              <div data-testid="context-item-text" style={quoteStyle}>
                {item.text}
              </div>
            ) : null}
            <div style={{ display: "flex", alignItems: "flex-start", gap: 6 }}>
              <StickyNote size={13} aria-hidden style={{ flexShrink: 0, marginTop: 2, color: "var(--text-muted)" }} />
              <span style={{ flex: 1, minWidth: 0 }}>
                <InlineEdit
                  value={item.note ?? ""}
                  multiline
                  allowEmpty
                  placeholder="Add a note for the agent…"
                  ariaLabel="Note"
                  testId="context-panel-note"
                  displayStyle={{ color: item.note ? "var(--text-primary)" : "var(--text-muted)", whiteSpace: "pre-wrap", cursor: "text" }}
                  onCommit={(next) => {
                    if (next.trim() !== (item.note ?? "")) void setContextNote(item.id, next);
                  }}
                />
              </span>
            </div>
            <div style={{ display: "flex", gap: 14 }}>
              {item.sourceUri ? (
                <button type="button" data-testid="context-item-open" title={item.sourceUri} onClick={() => onOpenPage({ id: item.sourceUri! })} style={linkButton}>
                  Open
                </button>
              ) : null}
              <button type="button" data-testid="context-item-move" onClick={onMove} style={linkButton}>
                Move to…
              </button>
            </div>
          </div>
        </div>
      </div>
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

const bodyStyle: CSSProperties = { display: "flex", flexDirection: "column", fontSize: "var(--text-xs)", minHeight: "100%" };

// The main area's tab strip: the panel-header tint, a strong rule, tabs
// divided by rules, the open one tinted and underlined in the accent.
const tabStrip: CSSProperties = {
  display: "flex",
  flexShrink: 0,
  overflowX: "auto",
  overflowY: "hidden",
  scrollbarWidth: "none",
  borderBottom: "1px solid var(--border-strong)",
  background: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg)), var(--surface-card)",
};


const ellipsis = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 } as const;

const quoteStyle: CSSProperties = {
  color: "var(--text-secondary)",
  borderLeft: "2px solid var(--border-subtle)",
  paddingLeft: 8,
  whiteSpace: "pre-wrap",
  fontFamily: "var(--font-mono, monospace)",
  maxHeight: 220,
  overflow: "auto",
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
  flexShrink: 0,
} as const;
