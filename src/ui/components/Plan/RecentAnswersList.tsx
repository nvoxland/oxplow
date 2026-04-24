import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  archiveAgentTurn,
  listRecentInactiveTurns,
  subscribeTurnEvents,
  type AgentTurn,
} from "../../api.js";
import { reportUiError } from "../../ui-error.js";
import { ContextMenu } from "../ContextMenu.js";
import { sectionHeaderStyle, type PlanSectionKey } from "./plan-utils.js";

/**
 * "Recent answers" — a compact list of recently-closed `agent_turn` rows
 * that produced NO activity (pure Q&A / planning / review). Lets the user
 * re-read a question and its answer after the turn has ended, without
 * cluttering the Done section or keeping an open-turn row around.
 *
 * Each row shows the (truncated) prompt and a right-side archive icon
 * for one-click dismissal. Right-click opens a menu with Archive;
 * double-click opens a modal with the full prompt + answer plus
 * Archive / Close buttons.
 *
 * Feeds on `turn.changed` events: Stop closes the turn and the runtime
 * persists `produced_activity`, which the backing query filters on.
 */
export function RecentAnswersList({
  streamId,
  threadId,
  isSectionCollapsed,
  onToggleSectionCollapsed,
}: {
  streamId: string | null;
  threadId: string | null;
  /** Shared collapse-state accessors from PlanPane's useCollapsedSections
   *  so Recent answers behaves like the other Plan-pane sections. */
  isSectionCollapsed: (kind: PlanSectionKey) => boolean;
  onToggleSectionCollapsed: (kind: PlanSectionKey) => void;
}) {
  const [turns, setTurns] = useState<AgentTurn[]>([]);
  const [openTurn, setOpenTurn] = useState<AgentTurn | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; turn: AgentTurn } | null>(null);

  useEffect(() => {
    if (!threadId) { setTurns([]); return; }
    let cancelled = false;
    const refresh = () => {
      listRecentInactiveTurns(threadId, 10)
        .then((rows) => { if (!cancelled) setTurns(rows); })
        .catch((err) => reportUiError("Load recent answers", err));
    };
    refresh();
    const off = subscribeTurnEvents(streamId ?? "all", (event) => {
      if (event.threadId !== threadId) return;
      refresh();
    });
    return () => { cancelled = true; off(); };
  }, [streamId, threadId]);

  const archive = (turnId: string) => {
    archiveAgentTurn(turnId).catch((err) => reportUiError("Archive recent answer", err));
  };

  if (turns.length === 0) return null;
  const isCollapsed = isSectionCollapsed("recentAnswers");
  return (
    <div data-testid="plan-recent-answers">
      <div
        style={{ ...sectionHeaderStyle, display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}
        data-testid="plan-section-header-recentAnswers"
        onClick={() => onToggleSectionCollapsed("recentAnswers")}
      >
        <span style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 6 }}>
          <span>Recent answers</span>
          <span style={{ color: "var(--muted)", fontWeight: 400, letterSpacing: 0 }}>{turns.length}</span>
        </span>
      </div>
      {!isCollapsed ? turns.map((turn) => (
        <RecentAnswerRow
          key={turn.id}
          turn={turn}
          onOpen={() => setOpenTurn(turn)}
          onArchive={() => archive(turn.id)}
          onContextMenu={(event) => {
            event.preventDefault();
            setMenu({ x: event.clientX, y: event.clientY, turn });
          }}
        />
      )) : null}
      {openTurn ? (
        <RecentAnswerModal
          turn={openTurn}
          onClose={() => setOpenTurn(null)}
          onArchive={() => {
            archive(openTurn.id);
            setOpenTurn(null);
          }}
        />
      ) : null}
      {menu ? (
        <ContextMenu
          items={[{
            id: "archive",
            label: "Archive",
            enabled: true,
            run: () => { archive(menu.turn.id); setMenu(null); },
          }]}
          position={{ x: menu.x, y: menu.y }}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </div>
  );
}

function RecentAnswerRow({
  turn,
  onOpen,
  onArchive,
  onContextMenu,
}: {
  turn: AgentTurn;
  onOpen(): void;
  onArchive(): void;
  onContextMenu(event: React.MouseEvent): void;
}) {
  const promptPreview = truncate(turn.prompt, 60);
  return (
    <div
      data-testid="plan-recent-answer-row"
      onDoubleClick={onOpen}
      onContextMenu={onContextMenu}
      style={{
        padding: "4px 8px",
        borderLeft: "2px solid transparent",
        background: "transparent",
        fontSize: 12,
        cursor: "default",
        display: "flex",
        gap: 6,
        alignItems: "center",
      }}
      title="Double-click to view full Q&A"
    >
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {promptPreview}
      </span>
      <button
        type="button"
        onClick={(event) => { event.stopPropagation(); onArchive(); }}
        data-testid="plan-recent-answer-archive"
        title="Archive"
        aria-label="Archive"
        style={archiveIconBtnStyle}
      >
        🗄
      </button>
    </div>
  );
}

function RecentAnswerModal({
  turn,
  onClose,
  onArchive,
}: {
  turn: AgentTurn;
  onClose(): void;
  onArchive(): void;
}) {
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      data-testid="plan-recent-answer-modal"
      style={backdropStyle}
      role="dialog"
      aria-modal="true"
    >
      <div style={modalStyle} onClick={(e) => e.stopPropagation()}>
        <div style={headerStyle}>
          <span>Recent answer</span>
          <button
            type="button"
            onClick={onClose}
            style={closeBtnStyle}
            aria-label="Close"
            data-testid="plan-recent-answer-modal-close-x"
          >
            ×
          </button>
        </div>
        <div style={bodyStyle}>
          <div style={labelStyle}>Prompt</div>
          <textarea
            readOnly
            value={turn.prompt}
            style={textAreaStyle}
            data-testid="plan-recent-answer-prompt"
          />
          <div style={{ ...labelStyle, marginTop: 12 }}>Answer</div>
          <textarea
            readOnly
            value={turn.answer ?? "(no recorded answer)"}
            style={{ ...textAreaStyle, minHeight: 240 }}
            data-testid="plan-recent-answer-answer"
          />
        </div>
        <div style={footerStyle}>
          <button
            type="button"
            onClick={onArchive}
            style={footerBtnStyle}
            data-testid="plan-recent-answer-modal-archive"
          >
            Archive
          </button>
          <button
            type="button"
            onClick={onClose}
            style={{ ...footerBtnStyle, ...footerBtnPrimaryStyle }}
            data-testid="plan-recent-answer-modal-close"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return s.slice(0, n - 1) + "…";
}

const archiveIconBtnStyle: CSSProperties = {
  border: "none",
  background: "transparent",
  color: "var(--muted)",
  fontSize: 13,
  lineHeight: 1,
  cursor: "pointer",
  padding: "2px 4px",
};

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1100,
};

const modalStyle: CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 24px 60px rgba(0,0,0,0.5)",
  minWidth: 520,
  maxWidth: 720,
  maxHeight: "80vh",
  display: "flex",
  flexDirection: "column",
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "10px 14px",
  borderBottom: "1px solid var(--border)",
  fontSize: 13,
  fontWeight: 600,
  background: "var(--bg-1, var(--bg-2))",
};

const closeBtnStyle: CSSProperties = {
  border: "none",
  background: "transparent",
  color: "var(--muted)",
  fontSize: 20,
  lineHeight: 1,
  cursor: "pointer",
};

const bodyStyle: CSSProperties = {
  padding: 14,
  overflow: "auto",
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
};

const labelStyle: CSSProperties = {
  fontSize: 11,
  textTransform: "uppercase",
  letterSpacing: 0.6,
  color: "var(--muted)",
  marginBottom: 4,
};

const textAreaStyle: CSSProperties = {
  width: "100%",
  minHeight: 120,
  resize: "vertical",
  fontFamily: "var(--mono, monospace)",
  fontSize: 12,
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: 8,
  boxSizing: "border-box",
};

const footerStyle: CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 8,
  padding: "10px 14px",
  borderTop: "1px solid var(--border)",
  background: "var(--bg-1, var(--bg-2))",
};

const footerBtnStyle: CSSProperties = {
  padding: "6px 12px",
  fontSize: 12,
  border: "1px solid var(--border)",
  background: "var(--bg-2)",
  color: "var(--fg)",
  borderRadius: 4,
  cursor: "pointer",
};

const footerBtnPrimaryStyle: CSSProperties = {
  background: "var(--accent)",
  color: "#fff",
  borderColor: "var(--accent)",
};
