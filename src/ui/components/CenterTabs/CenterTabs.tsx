import type { ReactNode } from "react";
import type { AgentStatus } from "../../api.js";
import { AgentStatusDot } from "../AgentStatusDot.js";

export interface CenterTab {
  id: string;
  label: string;
  closable: boolean;
  render: () => ReactNode;
  agentStatus?: AgentStatus;
}

interface CenterTabsProps {
  tabs: CenterTab[];
  activeId: string;
  onActivate(id: string): void;
  onClose?(id: string): void;
  /** Rendered above the active tab's content. */
  header?: ReactNode;
}

export function CenterTabs({ tabs, activeId, onActivate, onClose, header }: CenterTabsProps) {
  const active = tabs.find((t) => t.id === activeId) ?? tabs[0] ?? null;
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-1)" }}>
        {tabs.map((tab) => {
          const isActive = tab.id === active?.id;
          return (
            <div
              key={tab.id}
              onClick={() => onActivate(tab.id)}
              style={{
                padding: "8px 12px",
                background: isActive ? "var(--bg)" : "transparent",
                color: isActive ? "var(--fg)" : "var(--muted)",
                borderRight: "1px solid var(--border)",
                borderBottom: isActive ? "2px solid var(--accent)" : "2px solid transparent",
                cursor: "pointer",
                fontSize: 12,
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
              <span>{tab.label}</span>
              {tab.closable && onClose ? (
                <button
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(tab.id);
                  }}
                  title={`Close ${tab.label}`}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: "var(--muted)",
                    cursor: "pointer",
                    padding: "0 2px",
                    fontSize: 14,
                    lineHeight: 1,
                  }}
                >
                  ×
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
      {header}
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
        {active ? active.render() : null}
      </div>
    </div>
  );
}
