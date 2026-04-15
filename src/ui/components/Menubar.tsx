import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import type { MenuGroup } from "../commands.js";

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
            <button
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
                {group.items.map((item) => (
                  <button
                    key={item.id}
                    onClick={() => {
                      if (!item.enabled) return;
                      item.run();
                      setOpenMenuId(null);
                    }}
                    disabled={!item.enabled}
                    style={{
                      ...menuItemStyle,
                      opacity: item.enabled ? 1 : 0.45,
                      cursor: item.enabled ? "pointer" : "default",
                    }}
                  >
                    <span style={checkStyle}>{item.checked ? "✓" : ""}</span>
                    <span style={{ flex: 1 }}>{item.label}</span>
                    <span style={shortcutStyle}>{item.shortcut ?? ""}</span>
                  </button>
                ))}
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
  minWidth: 240,
  display: "flex",
  flexDirection: "column",
  gap: 2,
  padding: 6,
  border: "1px solid var(--border)",
  borderRadius: 6,
  background: "var(--bg)",
  boxShadow: "0 8px 24px rgba(0, 0, 0, 0.35)",
  zIndex: 20,
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
