import type { CSSProperties } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import {
  MENU_MAX_HEIGHT,
  clampMenuPosition,
  submenuTopOffset,
} from "./contextMenuPosition.js";

import type { MenuItem, MenuPosition } from "../menu.js";
import { ICONS } from "../icons.js";

interface ContextMenuProps {
  items: MenuItem[];
  /** A title line above the items, naming what the menu acts on — for a
   *  trigger that doesn't show it (an icon with only initials). */
  header?: string;
  position: MenuPosition;
  onClose(): void;
  minWidth?: number;
  zIndex?: number;
}

interface MenuListProps {
  items: MenuItem[];
  /** One line above the items saying what they do (a submenu's). */
  header?: string;
  onAction?(): void;
  minWidth?: number;
  /** Draw the menu's own frame (border, chrome background); false when it
   *  sits inside a frame already (a `ContextMenu`, which frames its header
   *  and items together). */
  framed?: boolean;
}

export function ContextMenu({ items, header, position, onClose, minWidth = 220, zIndex = 1000 }: ContextMenuProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [resolvedPosition, setResolvedPosition] = useState(position);

  // NOTE: do not reset `resolvedPosition` back to the raw `position` here —
  // useLayoutEffect below clamps to viewport, and a separate useEffect that
  // wrote the unclamped value would fire AFTER layout effects and undo the
  // clamp on the very next microtask, letting menus slide off the bottom.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    setResolvedPosition(
      clampMenuPosition(
        position,
        { width: root.offsetWidth, height: root.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
      ),
    );
  }, [items, minWidth, position]);

  useEffect(() => {
    function handlePointerDown(event: MouseEvent) {
      if (rootRef.current?.contains(event.target as Node)) return;
      onClose();
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [onClose]);

  return (
    <div
      ref={rootRef}
      style={{
        ...menuStyle,
        position: "fixed",
        left: resolvedPosition.x,
        top: resolvedPosition.y,
        minWidth,
        zIndex,
      }}
      onMouseDown={(event) => event.stopPropagation()}
      // The menu is rendered inline inside the row it acts on, so a
      // menu-item click would otherwise bubble to the row's onClick —
      // selecting the row / closing the navigator slideout (tsk190).
      onClick={(event) => event.stopPropagation()}
    >
      {header ? (
        <div data-testid="context-menu-header" style={headerStyle} title={header}>
          {header}
        </div>
      ) : null}
      <MenuList items={items} onAction={onClose} minWidth={minWidth} framed={false} />
    </div>
  );
}

const headerStyle: CSSProperties = {
  padding: "6px 10px 5px",
  fontSize: "var(--text-xs)",
  fontWeight: 600,
  color: "var(--text-secondary)",
  borderBottom: "1px solid var(--border-subtle)",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
  maxWidth: 320,
};

export function MenuList({ items, header, onAction, minWidth = 220, framed = true }: MenuListProps) {
  const [openSubmenuId, setOpenSubmenuId] = useState<string | null>(null);
  const submenuRefs = useRef(new Map<string, HTMLButtonElement | null>());

  return (
    <div style={framed ? { ...menuStyle, position: "relative", minWidth } : listStyle}>
      {header ? (
        <div data-testid="context-submenu-header" style={submenuHeaderStyle}>
          {header}
        </div>
      ) : null}
      {items.map((item) => {
        if (item.separator) {
          return <div key={item.id} style={separatorStyle} />;
        }
        const hasSubmenu = !!item.submenu && item.submenu.length > 0;
        return (
          <div
            key={item.id}
            style={{ position: "relative" }}
            onMouseEnter={() => hasSubmenu && item.enabled && setOpenSubmenuId(item.id)}
            onMouseLeave={() => openSubmenuId === item.id && setOpenSubmenuId(null)}
          >
            <button type="button"
              data-testid={`menu-item-${item.id}`}
              ref={(el) => { submenuRefs.current.set(item.id, el); }}
              onClick={async () => {
                if (!item.enabled) return;
                if (hasSubmenu) {
                  // Open, never toggle: a mouse's hover opened it already,
                  // and its click mustn't close it again.
                  setOpenSubmenuId(item.id);
                  return;
                }
                try {
                  await Promise.resolve(item.run?.());
                } finally {
                  onAction?.();
                }
              }}
              disabled={!item.enabled}
              style={{
                ...menuItemStyle,
                opacity: item.enabled ? 1 : 0.45,
                cursor: item.enabled ? "pointer" : "default",
                background: openSubmenuId === item.id ? "var(--bg-2)" : "transparent",
              }}
            >
              <span style={checkStyle}>{item.checked ? "✓" : ""}</span>
              {item.swatch ? <span aria-hidden style={{ ...swatchStyle, background: item.swatch }} /> : null}
              {item.icon && ICONS[item.icon] ? <MenuIcon name={item.icon} /> : null}
              <span style={{ flex: 1 }}>{item.label}</span>
              <span style={shortcutStyle}>{hasSubmenu ? "▸" : item.shortcut ?? ""}</span>
            </button>
            {hasSubmenu && openSubmenuId === item.id ? (
              <Submenu
                anchor={submenuRefs.current.get(item.id) ?? null}
                items={item.submenu!}
                header={item.submenuHeader}
                onAction={onAction}
                minWidth={minWidth}
              />
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

/**
 * Nested submenu, placed on the window beside its item (`anchor`): the
 * list it opens from scrolls in place, and a submenu laid out inside it
 * would be clipped by it. It measures itself and shifts up when it would
 * run off the bottom (as the outer ContextMenu does), and opens to the
 * item's left when there's no room on its right.
 */
function Submenu({
  anchor,
  items,
  header,
  onAction,
  minWidth,
}: {
  anchor: HTMLElement | null;
  items: MenuItem[];
  /** What the submenu's items do, above them. */
  header?: string;
  onAction?(): void;
  minWidth: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const node = ref.current;
    if (!node || !anchor) return;
    const at = anchor.getBoundingClientRect();
    const own = node.getBoundingClientRect();
    const right = at.right + SUBMENU_GAP;
    const left =
      right + own.width > window.innerWidth && at.left - own.width - SUBMENU_GAP >= 0
        ? at.left - own.width - SUBMENU_GAP
        : right;
    const top = at.top + submenuTopOffset({ top: at.top, height: own.height }, window.innerHeight);
    setPlace({ left, top });
  }, [anchor, items, minWidth]);

  return (
    <div
      ref={ref}
      data-testid="context-submenu"
      style={{
        position: "fixed",
        left: place?.left ?? 0,
        top: place?.top ?? 0,
        // Measured before it's placed: hidden until then, so it never
        // flashes in the window's corner.
        visibility: place ? "visible" : "hidden",
        zIndex: 2,
      }}
    >
      <MenuList items={items} header={header} onAction={onAction} minWidth={minWidth} />
    </div>
  );
}

/** Between an item and its submenu. */
const SUBMENU_GAP = 2;

/** A submenu's line of intent, atop its items: it wraps (a sentence, not
 *  a name) and stays as narrow as the menu. */
const submenuHeaderStyle: CSSProperties = {
  padding: "4px 8px 6px",
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
  borderBottom: "1px solid var(--border-subtle)",
  marginBottom: 2,
  maxWidth: 220,
  whiteSpace: "normal",
};

/** An item's icon (`MenuItem.icon`), before its label. */
function MenuIcon({ name }: { name: string }) {
  const Icon = ICONS[name]!;
  return (
    <span aria-hidden data-testid="menu-item-icon" style={{ display: "inline-flex", marginRight: 6, color: "var(--text-secondary)" }}>
      <Icon size={14} />
    </span>
  );
}

const swatchStyle: CSSProperties = {
  width: 10,
  height: 10,
  borderRadius: 2,
  flexShrink: 0,
};

/** A list inside a frame: the items' layout without a second border. */
const listStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 2, position: "relative" };

const menuStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  // A menu longer than the window scrolls in place. Without this, position
  // alone can't save it: the root pins to the top margin and everything past
  // the fold is unreachable, and a submenu gets lifted until its FIRST items
  // are off the top (tsk146).
  maxHeight: MENU_MAX_HEIGHT,
  overflowY: "auto",
  gap: 2,
  padding: 6,
  // Chrome, as a tab's dialog is: the nav bar's tint, a strong edge.
  border: "1px solid var(--border-strong)",
  borderRadius: 6,
  background: "var(--surface-chrome)",
  boxShadow: "0 8px 24px rgba(0, 0, 0, 0.35)",
};

const menuItemStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  border: "none",
  borderRadius: 4,
  padding: "6px 8px",
  background: "transparent",
  color: "var(--fg)",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
  textAlign: "left",
};

const checkStyle: CSSProperties = {
  width: 12,
  color: "var(--accent)",
  flexShrink: 0,
};

const shortcutStyle: CSSProperties = {
  color: "var(--muted)",
  fontSize: 11,
  flexShrink: 0,
};

const separatorStyle: CSSProperties = {
  height: 1,
  margin: "4px 6px",
  background: "var(--border)",
  pointerEvents: "none",
};
