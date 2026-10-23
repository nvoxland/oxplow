/// One terminal on screen: its surface, its session, and what the session
/// says when it isn't running — stopped (Start), ended or failed to start
/// (Start Again / Retry), or unable to resume a saved conversation (Retry,
/// Start Fresh). The agent's parts are `agent`'s; a shell has none.
import { useRef, useState, type CSSProperties } from "react";

import { exitBanner } from "../components/terminalSession.js";
import { TerminalSurface } from "./TerminalSurface.js";
import { useShown } from "./useShown.js";
import { useTerminalSession } from "./useTerminalSession.js";

export interface AgentTerminal {
  /** The person stopped it: nothing spawns until Start. */
  stopped: boolean;
  /** What it is bound to: a change opens it again. */
  conversation: string | null;
  /** Start it (`oxplow.agent_session.start`). */
  onStart(): void;
  /** Begin a new conversation, the saved one dropped (confirmed here). */
  onStartFresh(): Promise<void>;
  /** Another saved conversation (where the harness lists them). */
  onChooseAnother?(): void;
  /** The person pressed Escape: the host records the interrupt. */
  onInterrupt(): void;
}

export interface TerminalViewProps {
  /** The PTY it shows: an `oxplow/terminal` or `oxplow/agent_session` URI. */
  uri: string;
  agent?: AgentTerminal;
  worktreePath?: string;
  onOpenFile?(absPath: string, line?: number, column?: number): void;
  isLinkablePath?(path: string): boolean;
}

export function TerminalView({ uri, agent, worktreePath, onOpenFile, isLinkablePath }: TerminalViewProps) {
  const boxRef = useRef<HTMLDivElement>(null);
  const shown = useShown(boxRef);
  const session = useTerminalSession(uri, {
    startOnActivate: !!agent,
    shown,
    stopped: agent?.stopped,
    conversation: agent?.conversation ?? null,
  });
  const detach = useRef<(() => void) | null>(null);
  const [confirmFresh, setConfirmFresh] = useState(false);
  const banner = session.ended ? exitBanner(session.ended, { canChoose: !!agent?.onChooseAnother }) : null;
  return (
    <div ref={boxRef} data-testid="terminal-view" data-uri={uri} style={boxStyle}>
      <TerminalSurface
        shown={shown}
        onOpened={(term) => {
          detach.current?.();
          detach.current = session.attach(term);
        }}
        onData={session.input}
        onBinary={session.binary}
        onResize={session.resize}
        onInterrupt={agent?.onInterrupt}
        worktreePath={worktreePath}
        cwd={session.cwd}
        onOpenFile={onOpenFile}
        isLinkablePath={isLinkablePath}
      />
      {session.stopped ? (
        <div data-testid="terminal-stopped" style={bannerStyle}>
          <span>Stopped. The saved conversation is kept.</span>
          {agent ? (
            <button type="button" data-testid="terminal-start" onClick={agent.onStart}>
              Start
            </button>
          ) : null}
        </div>
      ) : banner ? (
        <div data-testid="terminal-ended" style={bannerStyle}>
          <span>{banner.text}</span>
          {banner.actions.map((action) =>
            action === "choose-another" ? (
              <button key={action} type="button" data-testid="terminal-choose-another" onClick={() => agent?.onChooseAnother?.()}>
                Choose another…
              </button>
            ) : action === "start-fresh" ? (
              agent ? (
                <button
                  key={action}
                  type="button"
                  data-testid="terminal-start-fresh"
                  title="Begin a new conversation. The saved one is no longer resumed."
                  onClick={() => {
                    if (!confirmFresh) {
                      setConfirmFresh(true);
                      return;
                    }
                    setConfirmFresh(false);
                    void agent.onStartFresh().then(() => session.reopen());
                  }}
                >
                  {confirmFresh ? "Confirm: drop the saved conversation" : "Start Fresh"}
                </button>
              ) : null
            ) : (
              <button
                key={action}
                type="button"
                data-testid={action === "retry" ? "terminal-retry" : "terminal-start-again"}
                onClick={() => session.reopen()}
              >
                {action === "retry" ? "Retry" : "Start Again"}
              </button>
            ),
          )}
        </div>
      ) : null}
    </div>
  );
}

const boxStyle: CSSProperties = { position: "relative", flex: 1, minHeight: 0, display: "flex", flexDirection: "column" };
const bannerStyle: CSSProperties = {
  position: "absolute",
  left: 0,
  right: 0,
  bottom: 0,
  display: "flex",
  alignItems: "center",
  gap: 10,
  padding: "8px 12px",
  background: "var(--surface-card)",
  borderTop: "1px solid var(--border-subtle)",
  color: "var(--text-primary)",
  fontSize: 13,
  zIndex: 4,
};
