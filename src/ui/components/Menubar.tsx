import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import type { MenuGroup } from "../commands.js";
import { MenuList } from "./ContextMenu.js";

interface Props {
  groups: MenuGroup[];
}

export function Menubar({ groups }: Props) {
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handlePointerDown(event: MouseEvent) {
      if (rootRef.current?.contains(event.target as Node)) {
        return;
      }
      setOpenMenuId(null);
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOpenMenuId(null);
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, []);

  return (
    <div
      ref={rootRef}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 4,
        padding: "6px 10px",
        borderBottom: "1px solid var(--border)",
        background: "var(--bg-2)",
        position: "relative",
      }}
    >
      {groups.map((group) => {
        const open = openMenuId === group.id;
        return (
          <div
            key={group.id}
            style={{ position: "relative" }}
            onMouseEnter={() => {
              if (openMenuId) {
                setOpenMenuId(group.id);
              }
            }}
            >
              <button type="button"
                onClick={() => setOpenMenuId(open ? null : group.id)}
              style={{
                ...menuButtonStyle,
                background: open ? "var(--bg)" : "transparent",
                color: open ? "var(--fg)" : "var(--muted)",
              }}
              >
                {group.label}
              </button>
              {open ? (
                <div style={menuStyle}>
                  <MenuList items={group.items} onAction={() => setOpenMenuId(null)} minWidth={240} />
                </div>
              ) : null}
            </div>
        );
      })}
    </div>
  );
}

const menuButtonStyle: CSSProperties = {
  border: "none",
  borderRadius: 4,
  padding: "4px 10px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
};

const menuStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  left: 0,
  zIndex: 20,
};
