import type { CSSProperties, ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";

import type { Stream, Thread } from "../api.js";
import { AgentStatusDot } from "../components/AgentStatusDot.js";
import { InlineEdit } from "../components/InlineEdit.js";
import { MoreButton } from "../components/MoreButton.js";
import { Slideover } from "../components/Slideover.js";
import { useRowContextMenu, type MenuItems } from "../components/useRowContextMenu.js";
import type { HostProject } from "../host/hostApi.js";
import { STATE_WORDS } from "../host/HostHome.js";
import type { useStreamThreadMenus } from "../window/useStreamThreadMenus.js";
import type { ShellWindow } from "./shellWindow.js";

/** The host this page came from, for its project list: absent in the Mac
 *  app (its launcher lists projects). Injectable for tests. */
export interface SwitcherHost {
  origin: string;
  token: string;
  /** The project this page shows (the host's id). */
  current: string | null;
  listProjects(host: string, token: string): Promise<HostProject[]>;
  setHold(host: string, token: string, project: string, held: boolean): Promise<void>;
  go(path: string): void;
}

type Menus = ReturnType<typeof useStreamThreadMenus>;

/**
 * The tablet's switcher, from the context line: the host's projects (a
 * tap holds one and goes to it), then this project's streams with their
 * threads (a tap switches or selects). A row's long press or ☰ opens the
 * window's menu for it; its flows (rename, a new thread) are inline here.
 */
export function TabletSwitcher({
  open,
  onClose,
  win,
  menus,
  host,
}: {
  open: boolean;
  onClose(): void;
  win: ShellWindow;
  menus: Menus;
  host: SwitcherHost | null;
}) {
  const streams = [...win.streams].sort((a, b) => (a.kind === "primary" ? -1 : b.kind === "primary" ? 1 : 0));
  return (
    <Slideover open={open} onClose={onClose} title="Switch" testId="tablet-switcher">
      {host ? <Projects host={host} open={open} /> : null}
      <h2 style={heading}>Streams</h2>
      <ul style={list}>
        {streams.map((s) => (
          <li key={s.id}>
            <StreamRow stream={s} win={win} menus={menus} onDone={onClose} />
            <ul style={list}>
              {(win.threadStates[s.id]?.threads ?? []).map((t) => (
                <li key={t.id}>
                  <ThreadRow stream={s} thread={t} win={win} menus={menus} onDone={onClose} />
                </li>
              ))}
              {menus.pendingNewThreadFor === s.id ? (
                <li style={{ ...rowBase, paddingLeft: 28 }}>
                  <InlineEdit
                    value=""
                    startEditing
                    placeholder="New thread title"
                    testId={`switcher-new-thread-input-${s.id}`}
                    ariaLabel="New thread title"
                    onCommit={(title) => {
                      const t = title.trim();
                      if (!t) return menus.setPendingNewThreadFor(null);
                      // Made, it is listed under its stream: a tap selects it,
                      // as in the desktop's Navigator.
                      void win.createThread(s.id, t).then((made) => {
                        if (made) menus.setPendingNewThreadFor(null);
                      });
                    }}
                    onCancel={() => menus.setPendingNewThreadFor(null)}
                  />
                </li>
              ) : (
                <li>
                  <button type="button" style={{ ...addButton, paddingLeft: 28 }} data-testid={`switcher-add-thread-${s.id}`} onClick={() => menus.setPendingNewThreadFor(s.id)}>
                    + New thread
                  </button>
                </li>
              )}
            </ul>
          </li>
        ))}
      </ul>
      <button
        type="button"
        style={addButton}
        data-testid="switcher-new-stream"
        onClick={() => {
          win.openNewStream();
          onClose();
        }}
      >
        + New stream…
      </button>
    </Slideover>
  );
}

function StreamRow({ stream, win, menus, onDone }: { stream: Stream; win: ShellWindow; menus: Menus; onDone(): void }) {
  const renaming = menus.renaming?.kind === "stream" && menus.renaming.id === stream.id;
  return (
    <Row
      testId={`switcher-stream-${stream.id}`}
      menu={() => menus.streamMenu(stream)}
      header={stream.title}
      current={stream.id === win.stream?.id}
      onTap={() => {
        void win.switchStream(stream.id);
        onDone();
      }}
      status={win.streamStatuses[stream.id]}
      label={
        renaming ? (
          <Rename value={stream.title} onCommit={(t) => win.renameStream(stream.id, t)} onEnd={() => menus.setRenaming(null)} />
        ) : (
          <span style={{ fontWeight: 600 }}>{stream.title}</span>
        )
      }
    />
  );
}

function ThreadRow({ stream, thread, win, menus, onDone }: { stream: Stream; thread: Thread; win: ShellWindow; menus: Menus; onDone(): void }) {
  const renaming = menus.renaming?.kind === "thread" && menus.renaming.id === thread.id;
  return (
    <Row
      testId={`switcher-thread-${thread.id}`}
      indent
      menu={() => menus.threadMenu(thread)}
      header={thread.title}
      current={thread.id === win.selectedThread?.id}
      onTap={() => {
        void win.selectThread(stream.id, thread.id);
        onDone();
      }}
      status={win.agentStatuses[thread.id]}
      question={win.agentQuestions[thread.id]}
      label={renaming ? <Rename value={thread.title} onCommit={(t) => win.renameThread(thread.id, t)} onEnd={() => menus.setRenaming(null)} /> : <span>{thread.title}</span>}
    />
  );
}

/** A stream's or thread's name being renamed: Enter keeps it, Escape or an
 *  unchanged name drops it. */
function Rename({ value, onCommit, onEnd }: { value: string; onCommit(next: string): void | Promise<void>; onEnd(): void }) {
  return (
    // Its keys and clicks are the field's, not the row's (Enter would
    // select the thread).
    <span onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()} style={{ flex: 1 }}>
      <InlineEdit
        value={value}
        startEditing
        testId="switcher-rename-input"
        ariaLabel="New name"
        onCommit={(next) => {
          const t = next.trim();
          if (t && t !== value) void onCommit(t);
          onEnd();
        }}
        onCancel={onEnd}
      />
    </span>
  );
}

function Row({
  testId,
  menu,
  header,
  current,
  onTap,
  status,
  question,
  label,
  indent = false,
}: {
  testId: string;
  menu(): MenuItems;
  header: string;
  current: boolean;
  onTap(): void;
  status?: Parameters<typeof AgentStatusDot>[0]["status"];
  question?: string;
  label: ReactNode;
  indent?: boolean;
}) {
  const { menu: menuNode, openAt, ...handlers } = useRowContextMenu(menu, header);
  return (
    <div
      role="button"
      tabIndex={0}
      data-testid={testId}
      aria-current={current ? "true" : undefined}
      style={{ ...rowBase, ...(indent ? { paddingLeft: 28 } : null), ...(current ? currentRow : null) }}
      onClick={onTap}
      {...handlers}
      onKeyDown={(e) => {
        if (e.key === "Enter") onTap();
        else handlers.onKeyDown(e);
      }}
    >
      {status ? <AgentStatusDot status={status} question={question} /> : <span style={{ width: 8 }} />}
      <span style={rowLabel}>{label}</span>
      <MoreButton label={`Actions for ${header}`} onOpen={openAt} />
      {menuNode}
    </div>
  );
}

/** The host's projects: the one this page shows is current; a tap on
 *  another holds it (so it starts and stays) and goes there. */
function Projects({ host, open }: { host: SwitcherHost; open: boolean }) {
  const [projects, setProjects] = useState<HostProject[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const read = useCallback(async () => {
    try {
      setProjects(await host.listProjects(host.origin, host.token));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [host]);
  useEffect(() => {
    if (open) void read();
  }, [open, read]);
  const go = async (p: HostProject) => {
    try {
      await host.setHold(host.origin, host.token, p.id, true);
      host.go(`/p/${p.id}/`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <>
      <h2 style={heading}>Projects</h2>
      {error ? (
        <div role="alert" data-testid="switcher-projects-error" style={{ color: "var(--danger)", padding: "4px 0" }}>
          {error}
        </div>
      ) : null}
      {projects === null ? (
        <div style={{ color: "var(--text-muted)" }}>Loading…</div>
      ) : (
        <ul style={list}>
          {projects.map((p) => {
            const current = p.id === host.current;
            return (
              <li key={p.id}>
                <button
                  type="button"
                  data-testid={`switcher-project-${p.id}`}
                  aria-current={current ? "true" : undefined}
                  style={{ ...rowBase, ...(current ? currentRow : null), width: "100%", border: "none", textAlign: "left" }}
                  onClick={() => (current ? undefined : void go(p))}
                >
                  <span style={rowLabel}>{p.name}</span>
                  <span style={{ color: "var(--text-muted)" }}>{STATE_WORDS[p.state] ?? p.state}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </>
  );
}

const heading: CSSProperties = { fontSize: "var(--text-xs)", textTransform: "uppercase", color: "var(--text-secondary)", margin: "var(--space-4) 0 var(--space-1)" };
const list: CSSProperties = { listStyle: "none", margin: 0, padding: 0 };
const rowBase: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  minHeight: "var(--hit-min)",
  padding: "0 var(--space-2)",
  borderRadius: 6,
  background: "transparent",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
  cursor: "pointer",
};
const currentRow: CSSProperties = { background: "var(--surface-selected, var(--surface-card))" };
const rowLabel: CSSProperties = { flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" };
const addButton: CSSProperties = { ...rowBase, border: "none", width: "100%", textAlign: "left", color: "var(--text-secondary)" };
