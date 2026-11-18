/// The Context dock panel: what the person collected for agents, by basket.
/// **This Thread** first — the selected thread's own items, each led by a ×
/// to take it out, its note under it edited in place (Ctrl/Cmd+Enter saves,
/// empty removes it), and Clear, a button as the Alerts panel's — then each
/// named basket attached to this thread, marked Attached with Detach: the
/// thread's next prompt carries all of these. Then **Baskets**: every other
/// named basket, collapsed to its name and count, with Attach. A basket's
/// header right-click has Attach / Detach, Rename…, Clear and Delete; an
/// item's has Move to ▸ and Remove. The icon's badge is how many items the
/// thread's next prompt carries. The Context page is the panel header's arrow.

import { ChevronDown, ChevronRight, FileText, Quote, X } from "lucide-react";
import { useState, type MouseEvent, type ReactNode } from "react";

import { InlineEdit } from "../InlineEdit.js";
import {
  THREAD_BASKET,
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
  type NamedBasket,
} from "../../contextBasket.js";
import type { MenuItem } from "../../menu.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { ContextAddCard } from "./ContextAddCard.js";
import { plainTitle, useContextTitle } from "./contextTitle.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { dockButtonStyle } from "../Dock/dockStyle.js";

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

/** A row's name: its page's title when it was added without a label. */
function ItemTitle({ item }: { item: BasketItem }) {
  return <>{useContextTitle({ label: item.label, uri: item.sourceUri, text: item.text })}</>;
}

/** Why a basket can't be attached to `thread` now, or null when it can. */
export function attachDisabledReason(basket: NamedBasket, thread: string | null): string | null {
  if (!thread) return "Select a thread first";
  if (basket.items.length === 0) return "Nothing in it yet";
  return null;
}

export function ContextDockBody(_props: DockBodyProps) {
  const thread = useSelectedThreadId();
  const baskets = useBaskets(thread);
  const adding = usePendingAdd();
  const menu = useContextMenu();
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const [renaming, setRenaming] = useState<string | null>(null);
  const attached = baskets.named.filter((b) => thread != null && b.attachedTo === thread);
  const others = baskets.named.filter((b) => !attached.includes(b));

  const itemMenu = (e: MouseEvent, item: BasketItem) =>
    menu.open(e, [
      { id: "move", label: "Move to", enabled: true, submenu: moveTargets(baskets, thread, item) },
      { id: "remove", label: "Remove", enabled: true, run: () => removeFromContext(item.id) },
    ]);
  const basketMenu = (e: MouseEvent, basket: NamedBasket) => {
    const here = thread != null && basket.attachedTo === thread;
    const why = attachDisabledReason(basket, thread);
    const items: MenuItem[] = [
      here || basket.attachedTo
        ? { id: "detach", label: "Detach", enabled: true, run: () => detachBasket(basket.name) }
        : { id: "attach", label: "Attach to This Thread", enabled: why == null, hint: why ?? undefined, run: () => (thread ? attachBasket(basket.name, thread) : undefined) },
      { id: "rename", label: "Rename…", enabled: true, run: () => setRenaming(basket.name) },
      { id: "clear", label: "Clear", enabled: basket.items.length > 0, run: () => clearContext({ basket: basket.name }) },
      { id: "delete", label: "Delete", enabled: true, run: () => deleteBasket(basket.name) },
    ];
    menu.open(e, items);
  };

  const list = (items: BasketItem[]) =>
    items.map((item) => (
      <div key={item.id} data-testid="context-panel-item" onContextMenu={(e) => itemMenu(e, item)} style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
        <button type="button" data-testid="context-panel-remove" aria-label="Remove from context" title="Remove" onClick={() => void removeFromContext(item.id)} style={iconButton}>
          <X size={14} aria-hidden />
        </button>
        {item.kind === "text" ? <Quote size={14} aria-hidden /> : <FileText size={14} aria-hidden />}
        <span style={{ flex: 1, minWidth: 0 }}>
          <div style={ellipsis}>
            <ItemTitle item={item} />
          </div>
          {item.kind === "text" && item.text ? <div style={{ ...ellipsis, color: "var(--text-muted)" }}>{item.text.slice(0, 200)}</div> : null}
          <InlineEdit
            value={item.note ?? ""}
            multiline
            allowEmpty
            placeholder="Add a note…"
            ariaLabel="Note"
            testId="context-panel-note"
            displayStyle={{ color: item.note ? "var(--text-secondary)" : "var(--text-muted)", whiteSpace: "pre-wrap", cursor: "text" }}
            onCommit={(next) => {
              if (next.trim() !== (item.note ?? "")) void setContextNote(item.id, next);
            }}
          />
        </span>
      </div>
    ));

  const header = (basket: NamedBasket, expanded: boolean | null, trailing: ReactNode) => (
    <div data-testid="context-basket" data-basket={basket.name} onContextMenu={(e) => basketMenu(e, basket)} style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 4 }}>
      {expanded == null ? null : (
        <button
          type="button"
          data-testid="context-basket-toggle"
          aria-label={expanded ? "Collapse" : "Expand"}
          aria-expanded={expanded}
          onClick={() =>
            setOpen((prev) => {
              const next = new Set(prev);
              if (next.has(basket.name)) next.delete(basket.name);
              else next.add(basket.name);
              return next;
            })
          }
          style={iconButton}
        >
          {expanded ? <ChevronDown size={14} aria-hidden /> : <ChevronRight size={14} aria-hidden />}
        </button>
      )}
      <span style={{ flex: 1, minWidth: 0, fontWeight: 600, ...ellipsis }}>
        {renaming === basket.name ? (
          <InlineEdit
            value={basket.name}
            startEditing
            ariaLabel="Basket name"
            testId="context-basket-rename"
            onCommit={(next) => {
              setRenaming(null);
              if (next.trim() && next.trim() !== basket.name) void renameBasket(basket.name, next.trim());
            }}
            onCancel={() => setRenaming(null)}
          />
        ) : (
          <>
            {basket.name} <span style={{ color: "var(--text-muted)", fontWeight: 400 }}>{basket.items.length}</span>
          </>
        )}
      </span>
      {trailing}
    </div>
  );

  const empty = !adding && baskets.own.length === 0 && baskets.named.length === 0;
  return (
    <div data-testid="context-panel" style={{ display: "flex", flexDirection: "column", gap: 6, padding: "8px 12px", fontSize: "var(--text-xs)" }}>
      {adding ? <ContextAddCard add={adding} baskets={baskets} /> : null}
      {empty ? (
        <div data-testid="context-panel-empty" style={{ color: "var(--text-muted)" }}>
          No context added yet
        </div>
      ) : (
        <>
          {thread ? (
            <>
              <div data-testid="context-panel-thread" style={sectionTitle}>
                {THREAD_BASKET}
              </div>
              {baskets.own.length === 0 ? (adding ? null : <div style={{ color: "var(--text-muted)" }}>Nothing yet</div>) : list(baskets.own)}
              {baskets.own.length > 0 ? (
                <div style={{ display: "flex", marginTop: 2 }}>
                  <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-panel-clear" title="Take everything out of This Thread" onClick={() => void clearContext({ thread })}>
                    Clear
                  </button>
                </div>
              ) : null}
              {attached.map((basket) => (
                <div key={basket.id} data-testid="context-basket-attached" style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {header(
                    basket,
                    null,
                    <>
                      <span style={{ color: "var(--accent)" }}>Attached</span>
                      <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-basket-detach" onClick={() => void detachBasket(basket.name)}>
                        Detach
                      </button>
                    </>,
                  )}
                  {list(basket.items)}
                </div>
              ))}
            </>
          ) : null}
          {others.length > 0 ? (
            <div data-testid="context-panel-baskets" style={{ ...sectionTitle, marginTop: thread ? 8 : 0 }}>
              Baskets
            </div>
          ) : null}
          {others.map((basket) => {
            const expanded = open.has(basket.name);
            const why = attachDisabledReason(basket, thread);
            return (
              <div key={basket.id} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                {header(
                  basket,
                  expanded,
                  <>
                    {basket.attachedTo ? <span style={{ color: "var(--text-muted)" }}>attached elsewhere</span> : null}
                    <button
                      type="button"
                      className="ox-button"
                      style={dockButtonStyle}
                      data-testid="context-basket-attach"
                      disabled={why != null}
                      title={why ?? "This thread's next prompt carries it, then empties it"}
                      onClick={() => (thread ? void attachBasket(basket.name, thread) : undefined)}
                    >
                      Attach
                    </button>
                  </>,
                )}
                {expanded ? list(basket.items) : null}
              </div>
            );
          })}
        </>
      )}
      {menu.menu}
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

const ellipsis = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" } as const;

const sectionTitle = {
  color: "var(--text-muted)",
  textTransform: "uppercase",
  letterSpacing: "0.04em",
  fontSize: "var(--text-xs)",
} as const;

const iconButton = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-muted)",
  cursor: "pointer",
  fontSize: "inherit",
  display: "inline-flex",
} as const;
