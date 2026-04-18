import type { CSSProperties } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { MenuItem, MenuPosition } from "../menu.js";

interface ContextMenuProps {
  items: MenuItem[];
  position: MenuPosition;
  onClose(): void;
  minWidth?: number;
}

interface MenuListProps {
  items: MenuItem[];
  onAction?(): void;
  minWidth?: number;
}

export function ContextMenu({ items, position, onClose, minWidth = 220 }: ContextMenuProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [resolvedPosition, setResolvedPosition] = useState(position);

  useEffect(() => {
    setResolvedPosition(position);
  }, [position]);

  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const maxX = Math.max(8, window.innerWidth - root.offsetWidth - 8);
    const maxY = Math.max(8, window.innerHeight - root.offsetHeight - 8);
    setResolvedPosition({
      x: Math.min(Math.max(8, position.x), maxX),
      y: Math.min(Math.max(8, position.y), maxY),
    });
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
        zIndex: 1000,
      }}
      onMouseDown={(event) => event.stopPropagation()}
    >
      <MenuList items={items} onAction={onClose} minWidth={minWidth} />
    </div>
  );
}

export function MenuList({ items, onAction, minWidth = 220 }: MenuListProps) {
  const [openSubmenuId, setOpenSubmenuId] = useState<string | null>(null);
  const submenuRefs = useRef(new Map<string, HTMLButtonElement | null>());

  return (
    <div style={{ ...menuStyle, position: "relative", minWidth }}>
      {items.map((item) => {
        const hasSubmenu = !!item.submenu && item.submenu.length > 0;
        return (
          <div
            key={item.id}
            style={{ position: "relative" }}
            onMouseEnter={() => hasSubmenu && item.enabled && setOpenSubmenuId(item.id)}
            onMouseLeave={() => openSubmenuId === item.id && setOpenSubmenuId(null)}
          >
            <button
              ref={(el) => { submenuRefs.current.set(item.id, el); }}
              onClick={async () => {
                if (!item.enabled) return;
                if (hasSubmenu) {
                  setOpenSubmenuId((prev) => (prev === item.id ? null : item.id));
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
                background: openSubmenuId === item.id ? "var(--bg-2)" : undefined,
              }}
            >
              <span style={checkStyle}>{item.checked ? "✓" : ""}</span>
              <span style={{ flex: 1 }}>{item.label}</span>
              <span style={shortcutStyle}>{hasSubmenu ? "▸" : item.shortcut ?? ""}</span>
            </button>
            {hasSubmenu && openSubmenuId === item.id ? (
              <div
                style={{
                  position: "absolute",
                  top: 0,
                  left: "100%",
                  marginLeft: 2,
                  zIndex: 2,
                }}
              >
                <MenuList items={item.submenu!} onAction={onAction} minWidth={minWidth} />
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

const menuStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 2,
  padding: 6,
  border: "1px solid var(--border)",
  borderRadius: 6,
  background: "var(--bg)",
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
  fontSize: 12,
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
