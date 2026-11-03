/// The nav bar's modes: the pages that can draw the tab's URI, swapped in
/// one click (usability.md "Actions are data; core draws them": the mode
/// is what a person comes to a README or a CSV to pick). Two or three are
/// a segmented control, the one drawing pressed; more are one button named
/// for the current mode that opens the list. Both are the ☰'s Open With
/// pick (`openWithOf`): the default renderer drops the key. Cmd+Shift+V
/// cycles them (`navigationKeys.ts`).
import type { CSSProperties } from "react";
import { useRef, useState } from "react";
import { ChevronDown } from "lucide-react";

import { ContextMenu } from "../components/ContextMenu.js";
import type { MenuPosition } from "../menu.js";
import type { OpenWithProps } from "./pageMenuItems.js";

/** Up to this many modes draw side by side; more fold into a list. */
const SEGMENTS = 3;

const MENU_WIDTH = 200;

export function PageModes({ openWith, buttonStyle }: { openWith: OpenWithProps; buttonStyle: CSSProperties }) {
  const [at, setAt] = useState<MenuPosition | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  const current = openWith.options.find((o) => o.name === openWith.current);
  if (openWith.options.length <= SEGMENTS) {
    return (
      <div role="group" aria-label="View as" data-testid="page-nav-mode-group" style={{ display: "inline-flex" }}>
        {openWith.options.map((o, i) => {
          const pressed = o.name === openWith.current;
          return (
            <button
              key={o.name}
              type="button"
              data-testid={`page-nav-mode-${o.name}`}
              title={pressed ? `Shown as ${o.title}` : `View as ${o.title}`}
              aria-pressed={pressed}
              onClick={() => {
                if (!pressed) openWith.pick(o.name);
              }}
              style={{
                ...buttonStyle,
                borderRadius: i === 0 ? "4px 0 0 4px" : i === openWith.options.length - 1 ? "0 4px 4px 0" : 0,
                marginLeft: i === 0 ? 0 : -1,
                background: pressed ? "var(--surface-elevated)" : "transparent",
                color: pressed ? "var(--text-primary)" : "var(--text-secondary)",
                fontWeight: pressed ? 600 : 400,
              }}
            >
              {o.title}
            </button>
          );
        })}
      </div>
    );
  }
  const toggle = () => {
    if (at) {
      setAt(null);
      return;
    }
    const r = button.current?.getBoundingClientRect();
    if (!r) return;
    setAt({ x: Math.max(8, r.right - MENU_WIDTH), y: r.bottom + 4 });
  };
  return (
    <div style={{ position: "relative", display: "inline-flex" }}>
      <button
        ref={button}
        type="button"
        data-testid="page-nav-modes"
        title="View as"
        aria-haspopup="menu"
        aria-expanded={at !== null}
        onMouseDown={(e) => {
          if (at) e.stopPropagation();
        }}
        onClick={toggle}
        style={{ ...buttonStyle, gap: 4 }}
      >
        {current?.title ?? "View as"}
        <ChevronDown size={14} aria-hidden />
      </button>
      {at ? (
        <ContextMenu
          items={openWith.options.map((o) => ({
            id: `page-mode-${o.name}`,
            label: o.title,
            enabled: true,
            checked: o.name === openWith.current,
            run: () => {
              if (o.name !== openWith.current) openWith.pick(o.name);
            },
          }))}
          position={at}
          minWidth={MENU_WIDTH}
          onClose={() => setAt(null)}
          zIndex={20}
        />
      ) : null}
    </div>
  );
}
