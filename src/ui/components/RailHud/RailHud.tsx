import type { CSSProperties } from "react";
import { useMemo } from "react";
import type { AgentStatus, BacklogState, ThreadWorkState, WorkItem } from "../../api.js";
import type { TabRef } from "../../tabs/tabState.js";
import { dashboardRef, indexRef, fileRef, workItemRef } from "../../tabs/pageRefs.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import { computeActiveItem, computeUpNext, sortRecentFiles, type RecentFileEntry } from "./sections.js";

export interface RailHudProps {
  threadId: string | null;
  threadWork: ThreadWorkState | null;
  backlog: BacklogState | null;
  agentStatus: AgentStatus;
  recentFiles: RecentFileEntry[];
  /** Open a page (or focus if already open) in the active thread's tab area. */
  onOpenPage(ref: TabRef): void;
  /** Optional: invoked when the user clicks the search affordance. */
  onOpenSearch?(): void;
}

/**
 * Heads-up display rail. Always visible on the left; passive by design —
 * never auto-opens tabs. Sections only render when they have content.
 *
 * - Search button (placeholder for ⌘K palette)
 * - Active item summary
 * - Since you last looked  (TBD; placeholder for now)
 * - Up next
 * - Recent files
 * - Pages directory
 */
export function RailHud({
  threadId,
  threadWork,
  backlog,
  agentStatus,
  recentFiles,
  onOpenPage,
  onOpenSearch,
}: RailHudProps) {
  const activeItem = useMemo(() => computeActiveItem(threadWork), [threadWork]);
  const upNext = useMemo(() => computeUpNext(threadWork, 5), [threadWork]);
  const recents = useMemo(() => sortRecentFiles(recentFiles, 6), [recentFiles]);
  const backlogReadyCount = backlog?.items.filter((i) => i.status === "ready").length ?? 0;

  return (
    <aside
      data-testid="rail-hud"
      style={{
        width: 260,
        flexShrink: 0,
        height: "100%",
        background: "var(--surface-rail)",
        borderRight: "1px solid var(--border-subtle)",
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        overflow: "auto",
      }}
    >
      <SearchTrigger onOpenSearch={onOpenSearch} />

      {threadId ? (
        <ActiveItemSection
          item={activeItem}
          agentStatus={agentStatus}
          onOpenPage={onOpenPage}
        />
      ) : null}

      {upNext.length > 0 ? (
        <UpNextSection items={upNext} onOpenPage={onOpenPage} />
      ) : null}

      {recents.length > 0 ? (
        <RecentFilesSection entries={recents} onOpenPage={onOpenPage} />
      ) : null}

      <PagesDirectory onOpenPage={onOpenPage} backlogReadyCount={backlogReadyCount} />
    </aside>
  );
}

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: "12px 14px 4px",
        fontSize: 11,
        fontWeight: 600,
        color: "var(--text-secondary)",
        textTransform: "uppercase",
        letterSpacing: 0.4,
      }}
    >
      {children}
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 14px",
  fontSize: 13,
  color: "var(--text-primary)",
  cursor: "pointer",
  border: "none",
  background: "transparent",
  textAlign: "left",
  width: "100%",
  borderRadius: 0,
};

function rowHoverStyle(): CSSProperties {
  return { ...rowStyle };
}

function SearchTrigger({ onOpenSearch }: { onOpenSearch?: () => void }) {
  return (
    <div style={{ padding: "12px 12px 8px" }}>
      <button
        type="button"
        data-testid="rail-search"
        onClick={onOpenSearch}
        style={{
          width: "100%",
          padding: "8px 10px",
          background: "var(--surface-card)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          color: "var(--text-secondary)",
          fontSize: 13,
          textAlign: "left",
          cursor: onOpenSearch ? "pointer" : "default",
          display: "flex",
          alignItems: "center",
          gap: 8,
        }}
      >
        <span aria-hidden style={{ opacity: 0.7 }}>🔍</span>
        <span style={{ flex: 1 }}>Search…</span>
        <kbd
          style={{
            fontSize: 10,
            color: "var(--text-muted)",
            background: "var(--surface-tab-inactive)",
            padding: "1px 5px",
            borderRadius: 3,
            border: "1px solid var(--border-subtle)",
          }}
        >
          ⌘K
        </kbd>
      </button>
    </div>
  );
}

function ActiveItemSection({
  item,
  agentStatus,
  onOpenPage,
}: {
  item: WorkItem | null;
  agentStatus: AgentStatus;
  onOpenPage(ref: TabRef): void;
}) {
  if (!item) {
    return (
      <>
        <SectionHeading>Active item</SectionHeading>
        <div
          data-testid="rail-active-empty"
          style={{
            padding: "4px 14px 12px",
            fontSize: 12,
            color: "var(--text-muted)",
          }}
        >
          No item in progress.
        </div>
      </>
    );
  }
  return (
    <>
      <SectionHeading>Active item</SectionHeading>
      <button
        type="button"
        data-testid="rail-active-item"
        onClick={() => onOpenPage(workItemRef(item.id))}
        draggable
        onDragStart={(ev) => setContextRefDrag(ev, {
          kind: "work-item",
          itemId: item.id,
          title: item.title,
          status: item.status,
        })}
        style={{
          ...rowStyle,
          flexDirection: "column",
          alignItems: "stretch",
          padding: "4px 14px 12px",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            color: "var(--text-primary)",
            fontWeight: 500,
            fontSize: 13,
            marginBottom: 4,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          <AgentStatusDot status={agentStatus} />
          <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{item.title}</span>
        </div>
        <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>
          {humanStatus(agentStatus)} · {item.kind}
        </span>
      </button>
    </>
  );
}

function UpNextSection({
  items,
  onOpenPage,
}: {
  items: WorkItem[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Up next</SectionHeading>
      <div data-testid="rail-up-next" style={{ paddingBottom: 8 }}>
        {items.map((item) => (
          <button
            key={item.id}
            type="button"
            data-testid={`rail-up-next-item-${item.id}`}
            onClick={() => onOpenPage(workItemRef(item.id))}
            draggable
            onDragStart={(ev) => setContextRefDrag(ev, {
              kind: "work-item",
              itemId: item.id,
              title: item.title,
              status: item.status,
            })}
            style={rowHoverStyle()}
          >
            <span style={{ color: "var(--text-muted)", fontSize: 11 }}>☐</span>
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {item.title}
            </span>
          </button>
        ))}
      </div>
    </>
  );
}

function RecentFilesSection({
  entries,
  onOpenPage,
}: {
  entries: RecentFileEntry[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Recent files</SectionHeading>
      <div data-testid="rail-recent-files" style={{ paddingBottom: 8 }}>
        {entries.map((e) => {
          const basename = e.path.split("/").pop() ?? e.path;
          return (
            <button
              key={e.path}
              type="button"
              data-testid={`rail-recent-file-${e.path}`}
              title={e.path}
              onClick={() => onOpenPage(fileRef(e.path))}
              draggable
              onDragStart={(ev) => setContextRefDrag(ev, { kind: "file", path: e.path })}
              style={rowHoverStyle()}
            >
              <span style={{ color: "var(--text-muted)", fontSize: 11 }}>📄</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {basename}
              </span>
            </button>
          );
        })}
      </div>
    </>
  );
}

interface PageEntry {
  id: string;
  label: string;
  ref: TabRef;
  badge?: number;
}

function PagesDirectory({
  onOpenPage,
  backlogReadyCount,
}: {
  onOpenPage(ref: TabRef): void;
  backlogReadyCount: number;
}) {
  const entries: PageEntry[] = [
    { id: "start", label: "⌂  Start", ref: indexRef("start") },
    { id: "all-work", label: "📋  All work", ref: indexRef("all-work"), badge: backlogReadyCount > 0 ? backlogReadyCount : undefined },
    { id: "notes-index", label: "📒  Notes", ref: indexRef("notes-index") },
    { id: "files", label: "📁  Files", ref: indexRef("files") },
    { id: "code-quality", label: "⚠  Code quality", ref: indexRef("code-quality") },
    { id: "local-history", label: "⏱  Local history", ref: indexRef("local-history") },
    { id: "git-history", label: "🌿  Git history", ref: indexRef("git-history") },
    { id: "hook-events", label: "🪝  Hook events", ref: indexRef("hook-events") },
    { id: "subsystem-docs", label: "📑  Subsystem docs", ref: indexRef("subsystem-docs") },
    { id: "settings", label: "⚙  Settings", ref: indexRef("settings") },
    { id: "dashboard-planning", label: "📊  Planning", ref: dashboardRef("planning") },
    { id: "dashboard-review", label: "📊  Review", ref: dashboardRef("review") },
    { id: "dashboard-quality", label: "📊  Quality", ref: dashboardRef("quality") },
  ];
  return (
    <>
      <SectionHeading>Pages</SectionHeading>
      <div data-testid="rail-pages" style={{ paddingBottom: 12 }}>
        {entries.map((entry) => (
          <button
            key={entry.id}
            type="button"
            data-testid={`rail-page-${entry.id}`}
            onClick={() => onOpenPage(entry.ref)}
            style={rowHoverStyle()}
          >
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {entry.label}
            </span>
            {entry.badge ? (
              <span
                style={{
                  fontSize: 10,
                  color: "var(--text-secondary)",
                  background: "var(--surface-tab-inactive)",
                  padding: "1px 6px",
                  borderRadius: 999,
                }}
              >
                {entry.badge}
              </span>
            ) : null}
          </button>
        ))}
      </div>
    </>
  );
}

function humanStatus(status: AgentStatus): string {
  switch (status) {
    case "idle":
      return "idle";
    case "working":
      return "running";
    case "waiting":
      return "waiting on you";
    case "done":
      return "done";
    default:
      return status;
  }
}
