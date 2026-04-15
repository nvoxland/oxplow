import { useState } from "react";
import type { Stream } from "../api.js";
import { TerminalPane } from "./TerminalPane.js";
import { EditorPane } from "./EditorPane.js";

type TabId = "working" | "talking" | "editor";

export function MainTabs({ stream }: { stream: Stream }) {
  const [active, setActive] = useState<TabId>("working");

  const tabs: { id: TabId; label: string }[] = [
    { id: "working", label: "Working CC" },
    { id: "talking", label: "Talking CC" },
    { id: "editor", label: "Editor" },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-2)" }}>
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setActive(t.id)}
            style={{
              padding: "8px 16px",
              background: active === t.id ? "var(--bg)" : "transparent",
              color: active === t.id ? "var(--fg)" : "var(--muted)",
              border: "none",
              borderRight: "1px solid var(--border)",
              borderBottom: active === t.id ? "2px solid var(--accent)" : "2px solid transparent",
              cursor: "pointer",
              fontFamily: "inherit",
              fontSize: 12,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, position: "relative" }}>
        <PaneHost visible={active === "working"}>
          <TerminalPane paneTarget={stream.panes.working} />
        </PaneHost>
        <PaneHost visible={active === "talking"}>
          <TerminalPane paneTarget={stream.panes.talking} />
        </PaneHost>
        <PaneHost visible={active === "editor"}>
          <EditorPane stream={stream} />
        </PaneHost>
      </div>
    </div>
  );
}

function PaneHost({ visible, children }: { visible: boolean; children: React.ReactNode }) {
  return (
    <div
      style={{
        position: "absolute",
        inset: 0,
        display: visible ? "block" : "none",
      }}
    >
      {children}
    </div>
  );
}
