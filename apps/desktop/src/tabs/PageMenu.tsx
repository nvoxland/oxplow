/// The page nav bar's ☰: one menu at the bar's right edge holding what the
/// page can be acted on with (`pageMenuItems.ts` says what, in which order).
/// Its items run, open a submenu, or open a panel under the button
/// (Backlinks, Outbound, Pin to Dashboard). Hidden when it would be empty.
import type { CSSProperties, ReactNode } from "react";
import { useRef, useState } from "react";
import { Menu } from "lucide-react";

import { askAssistant } from "../components/Assistant/assistantSession.js";
import { addDisabledReason, addToContext, useSelectedThreadId } from "../contextBasket.js";
import { ContextMenu } from "../components/ContextMenu.js";
import { PagePinMenu, type PagePin } from "../pages/PagePinMenu.js";
import type { AskTarget } from "../components/Prompts/askTarget.js";
import { useRefOffers } from "../components/refCommands.js";
import type { CommandEntry } from "../components/quickOpenResults.js";
import { usePopoverDismiss } from "../components/usePopoverDismiss.js";
import type { PageAction } from "../kit/actions.js";
import type { MenuPosition } from "../menu.js";
import { pageMenuItems, type OpenWithProps, type PagePanel } from "./pageMenuItems.js";
import type { TabRef } from "./tabState.js";

export interface PageMenuProps {
  actions?: readonly PageAction[];
  ask?: AskTarget;
  /** The page's name, for its basket entry. */
  label?: string;
  openWith?: OpenWithProps;
  /** The page's "See also" links (a submenu) and how one opens. */
  seeAlso?: { links: ReadonlyArray<{ title: string; uri: string; description?: string | null }>; open(uri: string): void };
  backlinks?: { count: number; body: ReactNode };
  outbound?: { count: number; body: ReactNode };
  pin?: { tile: PagePin; onOpenPage(ref: TabRef): void; disabledReason?: string | null };
  buttonStyle: CSSProperties;
}

/** The widest the menu is, px; it opens leftward from the button. */
const MENU_WIDTH = 240;

export function PageMenu({ actions, ask, label, openWith, seeAlso, backlinks, outbound, pin, buttonStyle }: PageMenuProps) {
  const offersFor = useRefOffers();
  const [at, setAt] = useState<MenuPosition | null>(null);
  // The commands about the page's ref that apply now: asked when the menu
  // opens (the page's state may have changed since it last did).
  const [offers, setOffers] = useState<CommandEntry[]>([]);
  const [panel, setPanel] = useState<PagePanel | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  const boxRef = usePopoverDismiss<HTMLDivElement>(panel !== null, () => setPanel(null));

  const thread = useSelectedThreadId();
  const items = pageMenuItems({
    actions,
    ask: ask ? { askAssistant: () => void askAssistant({ kind: "page", uri: ask.ref, label: label ?? null }) } : undefined,
    context: ask
      ? { add: () => void addToContext({ uri: ask.ref, label }), disabledReason: addDisabledReason(thread) }
      : undefined,
    commands: ask ? offers : undefined,
    openWith,
    seeAlso,
    backlinks: backlinks?.count,
    outbound: outbound?.count,
    pin: pin ? { disabledReason: pin.disabledReason } : undefined,
    openPanel: setPanel,
  });
  if (items.length === 0) return null;

  const toggle = async () => {
    if (at) {
      setAt(null);
      return;
    }
    setPanel(null);
    const r = button.current?.getBoundingClientRect();
    if (!r) return;
    const position = { x: Math.max(8, r.right - MENU_WIDTH), y: r.bottom + 4 };
    if (ask) setOffers(await offersFor(ask.ref));
    setAt(position);
  };

  return (
    <div ref={boxRef} style={{ position: "relative", display: "inline-flex" }}>
      <button
        ref={button}
        type="button"
        data-testid="page-nav-menu"
        title="Actions for this page"
        aria-label="Actions for this page"
        aria-haspopup="menu"
        aria-expanded={at !== null || panel !== null}
        // The open menu closes on any press outside it; a press on its own
        // button is the toggle's to close, or the click would reopen it.
        onMouseDown={(e) => {
          if (at) e.stopPropagation();
        }}
        onClick={() => void toggle()}
        style={buttonStyle}
      >
        <Menu size={18} aria-hidden />
      </button>
      {at ? <ContextMenu items={items} position={at} minWidth={MENU_WIDTH} onClose={() => setAt(null)} zIndex={20} /> : null}
      {panel === "backlinks" && backlinks ? (
        <div data-testid="page-nav-backlinks-popover" style={panelStyle}>
          {backlinks.body}
        </div>
      ) : null}
      {panel === "outbound" && outbound ? (
        <div data-testid="page-nav-outbound-popover" style={panelStyle}>
          {outbound.body}
        </div>
      ) : null}
      {panel === "pin" && pin ? (
        <PagePinMenu
          tile={pin.tile}
          testId="page-pin"
          onOpenPage={pin.onOpenPage}
          onDone={() => setPanel(null)}
          style={{ ...panelStyle, padding: 4, minWidth: 200 }}
        />
      ) : null}
    </div>
  );
}

const panelStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  right: 0,
  minWidth: 280,
  maxWidth: 480,
  maxHeight: 360,
  overflow: "auto",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
  padding: 8,
  zIndex: 10,
};
