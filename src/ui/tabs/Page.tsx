import type { ReactNode } from "react";
import { useState } from "react";

export interface PageChip {
  label: string;
  /** Foreground color override (e.g. status color). */
  color?: string;
  /** Soft background color override. */
  background?: string;
  title?: string;
}

export interface PageProps {
  /** Page title shown in the header. */
  title: ReactNode;
  /** Optional kind/type label rendered as a small chip ("file", "work item"…). */
  kind?: string;
  /** Optional status / metadata chips rendered next to the kind. */
  chips?: PageChip[];
  /** Optional kebab actions — rendered as `⋯` popover. */
  actions?: ReactNode;
  /** Page body. */
  children: ReactNode;
  /** Optional backlinks region. When omitted, the panel header is hidden. */
  backlinks?: ReactNode;
  /** Test id applied to the page root. */
  testId?: string;
}

/**
 * Shared chrome for every page rendered inside a tab body. Provides:
 *  - A header (title + kind chip + status/metadata chips + actions slot)
 *  - The page body
 *  - A collapsible Backlinks panel anchored at the bottom
 *
 * The chrome reads only semantic CSS variables. Both light and dark
 * themes are styled by `public/index.html`.
 */
export function Page({ title, kind, chips, actions, children, backlinks, testId }: PageProps) {
  const [backlinksOpen, setBacklinksOpen] = useState(false);

  return (
    <div
      data-testid={testId ?? "page"}
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
        background: "var(--surface-card)",
        color: "var(--text-primary)",
      }}
    >
      <header
        data-testid="page-header"
        style={{
          padding: "14px 20px",
          borderBottom: "1px solid var(--border-subtle)",
          display: "flex",
          alignItems: "center",
          gap: 12,
          flexShrink: 0,
          minHeight: 56,
        }}
      >
        <div style={{ display: "flex", alignItems: "baseline", gap: 10, flex: 1, minWidth: 0 }}>
          <span
            data-testid="page-title"
            style={{
              fontSize: 17,
              fontWeight: 600,
              color: "var(--text-primary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {title}
          </span>
          {kind ? (
            <span
              data-testid="page-kind"
              style={{
                fontSize: 11,
                fontWeight: 500,
                color: "var(--text-secondary)",
                background: "var(--surface-tab-inactive)",
                padding: "2px 6px",
                borderRadius: 4,
                textTransform: "lowercase",
              }}
            >
              {kind}
            </span>
          ) : null}
          {chips?.map((chip, i) => (
            <span
              key={i}
              title={chip.title}
              style={{
                fontSize: 11,
                fontWeight: 500,
                color: chip.color ?? "var(--text-secondary)",
                background: chip.background ?? "transparent",
                padding: "2px 6px",
                borderRadius: 4,
                border: chip.background ? "none" : "1px solid var(--border-subtle)",
              }}
            >
              {chip.label}
            </span>
          ))}
        </div>
        {actions ? (
          <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>{actions}</div>
        ) : null}
      </header>
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "auto", display: "flex", flexDirection: "column" }}>
        {children}
      </div>
      {backlinks !== undefined ? (
        <div
          data-testid="page-backlinks"
          style={{
            borderTop: "1px solid var(--border-subtle)",
            background: "var(--surface-app)",
            flexShrink: 0,
          }}
        >
          <button
            type="button"
            data-testid="page-backlinks-toggle"
            onClick={() => setBacklinksOpen((v) => !v)}
            style={{
              border: "none",
              background: "transparent",
              color: "var(--text-secondary)",
              padding: "8px 16px",
              cursor: "pointer",
              fontSize: 12,
              width: "100%",
              textAlign: "left",
            }}
            aria-expanded={backlinksOpen}
          >
            {backlinksOpen ? "▾" : "▸"} Backlinks
          </button>
          {backlinksOpen ? (
            <div data-testid="page-backlinks-body" style={{ padding: "0 16px 12px", fontSize: 12 }}>
              {backlinks}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
