import { Page } from "../tabs/Page.js";
import { TerminalPane } from "../components/TerminalPane.js";
import type { Stream, Thread } from "../api.js";
import { recordUserInterrupt, startAgentSession, startAgentSessionFresh } from "../api.js";
import { useWorkspaceLinkIndex } from "../useWorkspaceLinkIndex.js";
import { ChatAgentView } from "../components/chat/ChatAgentView.js";
import { AnswersStrip } from "../components/Answers/AnswersStrip.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { TabRef } from "../tabs/tabState.js";
import type { AgentSessionRow } from "../agentSessions.js";
import { ConversationPicker } from "../components/ConversationPicker.js";
import type { PickerMode } from "../components/conversationRows.js";
import { useState } from "react";

interface AgentSessionPageProps {
  thread: Thread | null;
  /** The agent session the tab shows; `null` while the thread's sessions load. */
  session: AgentSessionRow | null;
  stream: Stream | null;
  visible: boolean;
  /** Click-through handler for file paths detected in terminal output. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
  /** ACP threads: open an agent edit's diff. */
  onOpenDiff?(spec: DiffSpec): void;
  /** ACP threads: open Settings (an agent that needs approval). */
  onOpenSettings?(): void;
  /** Open a page (an answer's links); the agent tab has no route context. */
  onOpenPage?(ref: TabRef): void;
  /** Its harness lists its saved conversations: the person may choose one. */
  canChooseConversation?: boolean;
  /** A chat session whose harness has a view of its own on its saved
   *  conversation: the label of the button that opens it. */
  nativeViewLabel?: string;
  /** The conversation picker open over the terminal, if one is. */
  picker?: PickerMode | null;
  /** Open (a mode) or close (`null`) the conversation picker. */
  onPicker?(mode: PickerMode | null): void;
}

/**
 * An agent session's tab: its terminal (a `terminal` session) or its ACP
 * chat (a `chat` one). Like every other page kind it renders inside the
 * shared Page chrome — configured to hide the nav bar (the terminal owns
 * its full height) and the header (no title row). Its tab is pinned and
 * follows the session's row (`reconcileSessionTabs`).
 */
export function AgentSessionPage({
  thread,
  session,
  stream,
  visible,
  onOpenFile,
  onOpenDiff,
  onOpenSettings,
  onOpenPage,
  canChooseConversation,
  nativeViewLabel,
  picker,
  onPicker,
}: AgentSessionPageProps) {
  // Bumped when the session moved to another conversation: its pane
  // opens that one once the stopped process has exited.
  const [restartKey, setRestartKey] = useState(0);
  // Only linkify terminal paths that are real workspace files/dirs, so dotted
  // words in agent prose (e.g. a collector name) aren't turned into broken links.
  const isLinkablePath = useWorkspaceLinkIndex(stream?.id);
  if (!thread) {
    return (
      <Page testId="page-agent" showNavBar={false} showHeader={false}>
        <div style={{ padding: 12, color: "var(--muted)" }}>No thread selected.</div>
      </Page>
    );
  }
  if (!session) {
    return (
      <Page testId="page-agent" showNavBar={false} showHeader={false}>
        <div style={{ padding: 12, color: "var(--muted)" }}>Loading the agent session…</div>
      </Page>
    );
  }
  // A chat session (an ACP agent, a Codex thread) talks to its agent as a
  // structured conversation, not a terminal. Keyed on the session like the
  // terminal below.
  if (session.kind === "chat") {
    return (
      <Page testId="page-agent" showNavBar={false} showHeader={false}>
        <ChatAgentView
          key={session.id}
          thread={thread}
          sessionId={session.id}
          worktreePath={stream?.worktree_path}
          visible={visible}
          onOpenDiff={onOpenDiff}
          onOpenFile={onOpenFile ? (p) => onOpenFile(p) : undefined}
          onOpenSettings={onOpenSettings}
          onOpenPage={onOpenPage}
          nativeViewLabel={nativeViewLabel}
        />
      </Page>
    );
  }
  return (
    <Page testId="page-agent" showNavBar={false} showHeader={false}>
      <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
        {/* What the agent showed with `oxplow_show_lens` (P6.C2); an ACP thread
          *  renders each answer inline in its transcript instead. */}
        <AnswersStrip key={thread.id} threadId={thread.id} onOpenPage={onOpenPage} />
        {/* A flex column all the way down: a percentage height under a flex
          *  item doesn't resolve, so the terminal kept its own height when
          *  the strip grew, was clipped, and never refit (tsk1042). */}
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", position: "relative" }}>
          {picker && onPicker ? (
            <ConversationPicker
              sessionId={session.id}
              mode={picker}
              onCancel={() => onPicker(null)}
              onChosen={() => {
                onPicker(null);
                setRestartKey((k) => k + 1);
              }}
            />
          ) : null}
          {/* Keyed on the session: another session's pane is another
            *  agent, so React must not reuse this xterm for it. */}
          <TerminalPane
            key={session.id}
            paneTarget={session.id}
            visible={visible}
            startOnActivate
            stopped={session.stoppedByUser}
            onStartFresh={() => startAgentSessionFresh(session.id, true)}
            onChooseAnother={canChooseConversation && onPicker ? () => onPicker("resume") : undefined}
            restartKey={restartKey}
            onStart={() => {
              void startAgentSession(session.id).catch(() => undefined);
            }}
            worktreePath={stream?.worktree_path}
            onOpenFile={onOpenFile}
            isLinkablePath={isLinkablePath}
            comments={
              stream
                ? {
                    streamId: stream.id,
                    threadId: thread.id,
                    targetKind: "agent_session",
                    targetId: session.id,
                  }
                : undefined
            }
            onUserInterrupt={() => {
              void recordUserInterrupt(session.id, thread.id, stream?.id ?? null);
            }}
          />
        </div>
      </div>
    </Page>
  );
}
