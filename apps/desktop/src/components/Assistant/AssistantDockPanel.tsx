/// The Assistant dock panel: a short-lived on-the-spot agent for questions
/// and small tasks about what the person is looking at. One session per
/// window (`assistantSession.ts`); opening it about something new starts a
/// new session when the current one has turns.

import { useCallback, useEffect, useMemo, useState } from "react";

import { querySql, undoCommand } from "../../api.js";
import { recordOpError } from "../../opErrors.js";
import { streamRowId } from "../../modelIds.js";
import { settingsRef } from "../../tabs/pageRefs.js";
import { typeOf } from "../../uri/uri.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { promptsAbout } from "../Prompts/promptModel.js";
import { usePromptCatalog } from "../Prompts/usePromptCatalog.js";
import { AssistantPromptBox } from "./AssistantPromptBox.js";
import { AssistantTranscript } from "./AssistantTranscript.js";
import {
  answerCard,
  cancelTurn,
  chooseModel,
  continueIn,
  dismissError,
  ensureAssistant,
  newAssistant,
  sendPrompt,
  setAllowChanges,
  useAssistant,
} from "./assistantSession.js";
import { badgeCount, isBusy, lastTurnUndoable, panelMode } from "./assistantModel.js";

export function useAssistantBadge(): DockBadge | null {
  const count = badgeCount(useAssistant().state);
  return count > 0 ? { count } : null;
}

const GENERIC_SUGGESTIONS = ["What changed in this project this week?", "What's in progress right now?"];

export function AssistantDockBody({ onOpenPage, streamId }: DockBodyProps) {
  const view = useAssistant();
  const { state } = view;
  const catalog = usePromptCatalog();
  const [undone, setUndone] = useState<ReadonlySet<number>>(new Set());
  const threads = useThreadsToContinueIn(streamId);

  // The panel opened with nothing asked: a bare session.
  useEffect(() => {
    void ensureAssistant();
  }, []);

  const mode = panelMode(state);
  const busy = isBusy(state);
  const aboutUri = view.context && "uri" in view.context ? view.context.uri : undefined;
  const suggestions = useMemo(() => {
    if (mode !== "empty") return [];
    const kind = aboutUri ? typeOf(aboutUri) : null;
    const about = kind ? promptsAbout(catalog, kind).map((p) => p.prompt) : [];
    return about.length > 0 ? about : GENERIC_SUGGESTIONS;
  }, [mode, aboutUri, catalog]);

  const undo = useCallback((auditId: number) => {
    void undoCommand(auditId, true).then(
      () => setUndone((s) => new Set(s).add(auditId)),
      (e: unknown) => recordOpError({ label: "Undo", message: e instanceof Error ? e.message : String(e) }),
    );
  }, []);
  const undoTurn = () => {
    for (const c of lastTurnUndoable(state)) if (!undone.has(c.auditId)) undo(c.auditId);
  };

  const turnUndoable = lastTurnUndoable(state).some((c) => !undone.has(c.auditId));
  const aboutLabel = view.context ? (view.context.label ?? ("uri" in view.context ? view.context.uri : undefined) ?? "the selection") : null;

  return (
    <div data-testid="assistant-panel" data-mode={mode} style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0, fontSize: "var(--text-xs)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "4px 10px", borderBottom: "1px solid var(--border-subtle)", flexWrap: "wrap" }}>
        {aboutLabel ? (
          <span data-testid="assistant-about" style={{ color: "var(--text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 260 }}>
            About {aboutLabel}
          </span>
        ) : null}
        <span style={{ flex: 1 }} />
        {state.models.length > 0 ? (
          <select
            data-testid="assistant-model"
            aria-label="Model"
            value={state.model ? `${state.model.provider}\u0000${state.model.model}` : ""}
            disabled={busy}
            onChange={(e) => {
              const [provider, model] = e.target.value.split("\u0000");
              void chooseModel(provider && model ? { provider, model } : null);
            }}
          >
            {state.model ? null : <option value="">Default</option>}
            {state.models.map((m) => (
              <option key={`${m.provider}\u0000${m.model}`} value={`${m.provider}\u0000${m.model}`}>
                {m.label}
              </option>
            ))}
          </select>
        ) : null}
        <label title="Let the Assistant change things; each change asks first">
          <input type="checkbox" data-testid="assistant-allow-changes" checked={state.allowChanges} disabled={busy} onChange={(e) => void setAllowChanges(e.target.checked)} /> Allow changes
        </label>
        {turnUndoable ? (
          <button type="button" data-testid="assistant-undo-turn" onClick={undoTurn} style={headerButton}>
            Undo this turn
          </button>
        ) : null}
        {threads.length > 0 && state.items.length > 0 ? (
          <select
            data-testid="assistant-continue"
            aria-label="Continue in session"
            value=""
            disabled={busy}
            onChange={(e) => e.target.value && void continueIn(e.target.value)}
          >
            <option value="">Continue in session…</option>
            {threads.map((t) => (
              <option key={t.id} value={t.id}>
                {t.title}
              </option>
            ))}
          </select>
        ) : null}
        <button type="button" data-testid="assistant-new" onClick={() => void newAssistant()} disabled={busy} style={headerButton}>
          New
        </button>
      </div>

      {view.error ? (
        <div data-testid="assistant-error" style={{ borderLeft: "3px solid var(--severity-high)", background: "var(--surface-card)", margin: 8, padding: "6px 10px" }}>
          {view.error}{" "}
          <button type="button" style={headerButton} onClick={dismissError}>
            Dismiss
          </button>
        </div>
      ) : null}

      {mode === "no_model" ? (
        <div data-testid="assistant-no-model" style={{ padding: "12px", color: "var(--text-secondary)" }}>
          <div>{state.problem?.message ?? "The Assistant has no model."}</div>
          <button type="button" data-testid="assistant-open-settings" onClick={() => onOpenPage(settingsRef("ai"))} style={link}>
            Choose a model in Settings → AI
          </button>
        </div>
      ) : view.opening ? (
        <div style={{ padding: 12, color: "var(--text-muted)" }}>Starting…</div>
      ) : (
        <AssistantTranscript items={state.items} undone={undone} onAnswer={(ask, allow) => void answerCard(ask, allow)} onUndo={undo} />
      )}

      <AssistantPromptBox
        busy={busy}
        disabled={mode === "no_model" || view.id === null}
        suggestions={suggestions}
        draft={view.draft}
        onSend={sendPrompt}
        onCancel={() => void cancelTurn()}
      />
    </div>
  );
}

/** The stream's open threads, to hand a conversation to. */
function useThreadsToContinueIn(streamId: string | null): Array<{ id: string; title: string }> {
  const [threads, setThreads] = useState<Array<{ id: string; title: string }>>([]);
  useEffect(() => {
    if (!streamId) {
      setThreads([]);
      return;
    }
    let cancelled = false;
    void querySql(
      "SELECT t.id, t.title FROM v_thread t WHERE t.stream_id = ?1 AND t.closed_at IS NULL AND t.archived_at IS NULL AND EXISTS (SELECT 1 FROM v_agent_session s WHERE s.thread_id = t.id AND s.closed_at IS NULL) ORDER BY t.id",
      [streamRowId(streamId)],
      200,
    ).then(
      (r) => {
        if (!cancelled) setThreads(r.rows.map((row) => ({ id: `thr${String(row[0])}`, title: String(row[1]) })));
      },
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, [streamId]);
  return threads;
}

const headerButton = {
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid var(--border-strong)",
  borderRadius: 4,
  padding: "1px 8px",
  fontSize: "inherit",
  cursor: "pointer",
} as const;

const link = { background: "none", border: "none", padding: 0, color: "var(--accent)", cursor: "pointer", fontSize: "inherit" } as const;
