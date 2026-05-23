import { Page } from "../tabs/Page.js";
import { TerminalPane } from "../components/TerminalPane.js";
import type { Stream, Thread } from "../api.js";
import { recordUserInterrupt } from "../api.js";

interface AgentPageProps {
  thread: Thread | null;
  stream: Stream | null;
  visible: boolean;
  transportMode: "direct" | "tmux";
  /** Click-through handler for file paths detected in terminal output. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
}

/**
 * Page wrapper for the agent terminal. Like every other page kind,
 * the agent renders inside the shared Page chrome — but configured
 * to hide the nav bar (the terminal owns its full height) and the
 * header (no title row). Tab-level non-closable behavior is enforced
 * at the host's centerTabs builder, not here.
 *
 * Wrapping in Page (instead of rendering TerminalPane directly)
 * makes the agent tab participate in the same architecture as every
 * other tab: a tab is a slot that holds a Page; the Page configures
 * what chrome it wants.
 */
export function AgentPage({ thread, stream, visible, transportMode, onOpenFile }: AgentPageProps) {
  if (!thread) {
    return (
      <Page testId="page-agent" showNavBar={false} showHeader={false}>
        <div style={{ padding: 12, color: "var(--muted)" }}>No thread selected.</div>
      </Page>
    );
  }
  return (
    <Page testId="page-agent" showNavBar={false} showHeader={false}>
      <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
        <div style={{ flex: 1, minHeight: 0 }}>
          {/* Key on thread.id so switching to a different thread
            *  remounts the terminal — pane_target alone collides
            *  ("working" for every thread), so without the key
            *  React reuses the same xterm + PTY session and the
            *  user keeps seeing the old thread's transcript even
            *  though the backend would happily attach a different
            *  per-thread session. */}
          <TerminalPane
            key={thread.id}
            paneTarget={thread.pane_target}
            visible={visible}
            transportMode={transportMode}
            worktreePath={stream?.worktree_path}
            onOpenFile={onOpenFile}
            comments={
              stream
                ? {
                    streamId: stream.id,
                    threadId: thread.id,
                    targetKind: "agent",
                    targetId: thread.id,
                  }
                : undefined
            }
            onUserInterrupt={() => {
              void recordUserInterrupt(thread.id, stream?.id ?? null);
            }}
          />
        </div>
      </div>
    </Page>
  );
}
