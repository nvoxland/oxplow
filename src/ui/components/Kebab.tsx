import type { CSSProperties } from "react";
import { useRef, useState } from "react";

import type { MenuItem } from "../menu.js";
import { ContextMenu } from "./ContextMenu.js";

/**
 * `⋯` button that opens the shared `ContextMenu` popover anchored to its
 * own bounding rect. Replaces every right-click → ContextMenu pairing
 * the IA redesign retired (per the inverted usability rules: per-row
 * actions = visible buttons + kebab popover, not hidden right-click).
 *
 * Uses the same `MenuItem[]` payload as the legacy right-click menus so
 * call sites only swap their handler; the menu items themselves are
 * unchanged.
 */
export function Kebab({
  items,
  label = "More actions",
  testId,
  size = 16,
  style,
}: {
  items: MenuItem[];
  label?: string;
  testId?: string;
  size?: number;
  style?: CSSProperties;
}) {
  const [open, setOpen] = useState<{ x: number; y: number } | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  function handleOpen() {
    const node = buttonRef.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    // Anchor menu under the kebab button, right-aligned to it.
    setOpen({ x: rect.right, y: rect.bottom + 4 });
  }

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-label={label}
        title={label}
        data-testid={testId}
        onClick={(e) => {
          e.stopPropagation();
          if (open) {
            setOpen(null);
          } else {
            handleOpen();
          }
        }}
        style={{
          width: size,
          height: size,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          background: "transparent",
          border: "none",
          color: "var(--muted)",
          cursor: "pointer",
          fontSize: size - 2,
          lineHeight: 1,
          padding: 0,
          ...(style ?? {}),
        }}
      >
        ⋯
      </button>
      {open ? (
        <ContextMenu
          items={items}
          position={open}
          onClose={() => setOpen(null)}
        />
      ) : null}
    </>
  );
}
