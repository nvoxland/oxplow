import { Page } from "../tabs/Page.js";
import { TerminalPane } from "../components/TerminalPane.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { Stream } from "../api.js";

interface TerminalPageProps {
  stream: Stream | null;
  visible: boolean;
  /** Click-through handler for file paths detected in terminal output. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
}

/**
 * A plain interactive shell rooted at the stream's worktree dir — the
 * same xterm surface as the agent terminal, but the backend spawns the
 * user's `$SHELL` for the `"shell"` pane target instead of the agent
 * command (see `commands/terminal.rs`). No `onUserInterrupt`: Escape is
 * an ordinary shell keystroke here, not an agent cancel.
 */
export function TerminalPage({ stream, visible, onOpenFile }: TerminalPageProps) {
  usePageTitle("Terminal");
  if (!stream) {
    return (
      <Page testId="page-terminal" title="Terminal" kind="terminal">
        <div style={{ padding: 12, color: "var(--muted)" }}>No project open.</div>
      </Page>
    );
  }
  return (
    <Page testId="page-terminal" title="Terminal" kind="terminal">
      <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
        <div style={{ flex: 1, minHeight: 0 }}>
          <TerminalPane
            paneTarget="shell"
            visible={visible}
            transportMode="direct"
            worktreePath={stream.worktree_path}
            onOpenFile={onOpenFile}
            comments={{
              streamId: stream.id,
              threadId: null,
              targetKind: "terminal",
              targetId: stream.id,
            }}
          />
        </div>
      </div>
    </Page>
  );
}
