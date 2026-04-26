import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { dashboardRef, indexRef } from "../tabs/pageRefs.js";

export interface StartPageProps {
  onOpenPage(ref: TabRef): void;
  /** Optional list of recent items to show in the "Recent in this thread" group. */
  recents?: Array<{ label: string; ref: TabRef }>;
}

interface StartGroup {
  heading: string;
  entries: Array<{ icon: string; label: string; ref: TabRef }>;
}

const GROUPS: StartGroup[] = [
  {
    heading: "Work",
    entries: [
      { icon: "📋", label: "Plan work", ref: indexRef("plan-work") },
      { icon: "✓", label: "Done work", ref: indexRef("done-work") },
      { icon: "📦", label: "Backlog", ref: indexRef("backlog") },
      { icon: "▣", label: "Archived", ref: indexRef("archived") },
    ],
  },
  {
    heading: "Knowledge",
    entries: [
      { icon: "📒", label: "Notes index", ref: indexRef("notes-index") },
      { icon: "📑", label: "Subsystem docs", ref: indexRef("subsystem-docs") },
    ],
  },
  {
    heading: "Code",
    entries: [
      { icon: "📁", label: "Files", ref: indexRef("files") },
      { icon: "⚠", label: "Code quality", ref: indexRef("code-quality") },
      { icon: "⏱", label: "Local history", ref: indexRef("local-history") },
      { icon: "🌿", label: "Git history", ref: indexRef("git-history") },
    ],
  },
  {
    heading: "Dashboards",
    entries: [
      { icon: "📊", label: "Planning", ref: dashboardRef("planning") },
      { icon: "📊", label: "Review", ref: dashboardRef("review") },
      { icon: "📊", label: "Quality", ref: dashboardRef("quality") },
    ],
  },
  {
    heading: "Configuration",
    entries: [
      { icon: "⚙", label: "Settings", ref: indexRef("settings") },
    ],
  },
];

/**
 * The "Start" page — a sitemap. The empty-state nudge for new users and a
 * keyboard-friendlier alternative to scanning the rail's Pages directory.
 */
export function StartPage({ onOpenPage, recents = [] }: StartPageProps) {
  return (
    <Page testId="page-start" title="Start" kind="overview">
      <div style={{ padding: "20px 24px", maxWidth: 720 }}>
        <p
          style={{
            color: "var(--text-secondary)",
            margin: "0 0 24px",
            fontSize: 14,
          }}
        >
          Where to go in this thread.
        </p>
        {GROUPS.map((group) => (
          <section key={group.heading} style={{ marginBottom: 24 }}>
            <h2
              style={{
                fontSize: 11,
                fontWeight: 600,
                color: "var(--text-secondary)",
                textTransform: "uppercase",
                letterSpacing: 0.4,
                margin: "0 0 8px",
              }}
            >
              {group.heading}
            </h2>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "repeat(auto-fill, minmax(180px, 1fr))",
                gap: 8,
              }}
            >
              {group.entries.map((entry) => (
                <button
                  key={entry.ref.id}
                  type="button"
                  data-testid={`start-page-entry-${entry.ref.id}`}
                  onClick={() => onOpenPage(entry.ref)}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    padding: "10px 12px",
                    background: "var(--surface-tab-inactive)",
                    color: "var(--text-primary)",
                    border: "1px solid var(--border-subtle)",
                    borderRadius: 8,
                    cursor: "pointer",
                    fontSize: 13,
                    textAlign: "left",
                  }}
                >
                  <span aria-hidden style={{ fontSize: 16 }}>{entry.icon}</span>
                  <span>{entry.label}</span>
                </button>
              ))}
            </div>
          </section>
        ))}
        {recents.length > 0 ? (
          <section>
            <h2
              style={{
                fontSize: 11,
                fontWeight: 600,
                color: "var(--text-secondary)",
                textTransform: "uppercase",
                letterSpacing: 0.4,
                margin: "16px 0 8px",
              }}
            >
              Recent in this thread
            </h2>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {recents.slice(0, 8).map((item) => (
                <button
                  key={item.ref.id}
                  type="button"
                  onClick={() => onOpenPage(item.ref)}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    padding: "6px 10px",
                    background: "transparent",
                    border: "1px solid transparent",
                    borderRadius: 6,
                    color: "var(--text-primary)",
                    cursor: "pointer",
                    fontSize: 13,
                    textAlign: "left",
                  }}
                >
                  {item.label}
                </button>
              ))}
            </div>
          </section>
        ) : null}
      </div>
    </Page>
  );
}
