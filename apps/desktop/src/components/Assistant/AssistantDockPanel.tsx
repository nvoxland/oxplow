/// The Assistant dock panel: a short-lived on-the-spot agent for questions
/// and small tasks about what the person is looking at. One session per
/// window (`assistantSession.ts`); opening it about something new starts a
/// new session when the current one has turns. Its toolbar (`DockToolbar`):
/// New, the model (a menu), **Allow Changes** (a switch: on, it may change
/// things, each change asking first), Undo This Turn, Continue in… (a menu
/// of the stream's sessions), and at the right what the conversation is
/// about. With no model it shows only how to choose one: Set Up the
/// Assistant opens AI Settings (foundation's page), where its role's model is chosen.

import { ChevronDown, CircleAlert, PencilLine, Plus, Sparkles, Undo2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type MouseEvent } from "react";

import { querySql, undoCommand } from "../../api.js";
import { recordOpError } from "../../opErrors.js";
import { streamRowId } from "../../modelIds.js";
import { aiSettingsRef } from "../../tabs/pageRefs.js";
import { typeOf } from "../../uri/uri.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { BarButton, DockToolbar } from "../Dock/DockToolbar.js";
import { useContextMenu } from "../useRowContextMenu.js";
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
  const menu = useContextMenu();

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

  const modelLabel = state.model ? (state.models.find((m) => m.provider === state.model!.provider && m.model === state.model!.model)?.label ?? state.model.model) : "Default model";
  const modelMenu = (e: MouseEvent) =>
    menu.open(e, [
      ...(state.model ? [] : [{ id: "model-default", label: "Default", enabled: true, checked: true }]),
      ...state.models.map((m) => ({
        id: `model-${m.provider}-${m.model}`,
        label: m.label,
        enabled: true,
        checked: state.model?.provider === m.provider && state.model?.model === m.model,
        run: () => void chooseModel({ provider: m.provider, model: m.model }),
      })),
    ]);
  const continueMenu = (e: MouseEvent) =>
    menu.open(
      e,
      threads.map((t) => ({ id: `continue-${t.id}`, label: t.title, enabled: true, run: () => void continueIn(t.id) })),
      "Hand this conversation to the session in",
    );

  if (mode === "no_model") {
    return (
      <div data-testid="assistant-panel" data-mode={mode} style={panelStyle}>
        <div data-testid="assistant-no-model" style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 10, padding: 16, textAlign: "center" }}>
          <Sparkles size={20} aria-hidden style={{ color: "var(--text-muted)" }} />
          <div style={{ color: "var(--text-primary)", fontSize: "var(--text-sm)", fontWeight: 600 }}>The Assistant needs a model</div>
          <div style={{ color: "var(--text-secondary)", maxWidth: 420 }}>{state.problem?.message ?? "Assign one to the Assistant role, or to Main, which it falls back to."}</div>
          <button type="button" className="ox-button ox-primary" data-testid="assistant-open-settings" onClick={() => onOpenPage(aiSettingsRef())}>
            Set Up the Assistant
          </button>
        </div>
      </div>
    );
  }

  return (
    <div data-testid="assistant-panel" data-mode={mode} style={panelStyle}>
      <DockToolbar testId="assistant-toolbar">
        <BarButton data-testid="assistant-new" onClick={() => void newAssistant()} disabled={busy} title="Start a new conversation">
          <Plus size={12} aria-hidden /> New
        </BarButton>
        {state.models.length > 0 ? (
          <BarButton data-testid="assistant-model" aria-label="Model" aria-haspopup="menu" disabled={busy} title="The model it uses" onClick={modelMenu}>
            {modelLabel} <ChevronDown size={11} aria-hidden />
          </BarButton>
        ) : null}
        <BarButton
          role="switch"
          aria-checked={state.allowChanges}
          data-testid="assistant-allow-changes"
          disabled={busy}
          title={state.allowChanges ? "It may change things; each change asks you first. Click to make it read only." : "Read only. Click to let it change things; each change asks you first."}
          onClick={() => void setAllowChanges(!state.allowChanges)}
          style={state.allowChanges ? { background: "var(--accent-soft-bg)", color: "var(--accent)", boxShadow: "inset 0 0 0 1px var(--accent)" } : undefined}
        >
          <PencilLine size={12} aria-hidden /> {state.allowChanges ? "Changes Allowed" : "Allow Changes"}
        </BarButton>
        {turnUndoable ? (
          <BarButton data-testid="assistant-undo-turn" onClick={undoTurn} title="Undo every change this turn made">
            <Undo2 size={12} aria-hidden /> Undo This Turn
          </BarButton>
        ) : null}
        {threads.length > 0 && state.items.length > 0 ? (
          <BarButton data-testid="assistant-continue" aria-haspopup="menu" disabled={busy} title="Hand this conversation to an agent session" onClick={continueMenu}>
            Continue in… <ChevronDown size={11} aria-hidden />
          </BarButton>
        ) : null}
        {aboutLabel ? (
          <span
            data-testid="assistant-about"
            title={`This conversation is about ${aboutLabel}. New starts one about nothing.`}
            style={{ marginLeft: "auto", padding: "1px 8px", borderRadius: 999, border: "1px solid var(--border-subtle)", color: "var(--text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 280 }}
          >
            About {aboutLabel}
          </span>
        ) : null}
      </DockToolbar>

      {view.error ? (
        <div data-testid="assistant-error" style={{ display: "flex", alignItems: "flex-start", gap: 8, padding: "6px 12px", borderBottom: "1px solid var(--border-subtle)" }}>
          <CircleAlert size={13} aria-label="Failed" style={{ flexShrink: 0, marginTop: 1, color: "var(--severity-critical)" }} />
          <span style={{ flex: 1, minWidth: 0, color: "var(--text-primary)" }}>{view.error}</span>
          <button type="button" aria-label="Dismiss" title="Dismiss" onClick={dismissError} style={iconButton}>
            <X size={14} aria-hidden />
          </button>
        </div>
      ) : null}

      {view.opening ? (
        <div style={{ flex: 1, padding: 12, color: "var(--text-muted)" }}>Starting…</div>
      ) : (
        <AssistantTranscript items={state.items} undone={undone} onAnswer={(ask, allow) => void answerCard(ask, allow)} onUndo={undo} />
      )}

      <AssistantPromptBox busy={busy} disabled={view.id === null} suggestions={suggestions} draft={view.draft} onSend={sendPrompt} onCancel={() => void cancelTurn()} />
      {menu.menu}
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

const panelStyle = { display: "flex", flexDirection: "column", height: "100%", minHeight: 0, fontSize: "var(--text-xs)" } as const;

const iconButton = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-muted)",
  cursor: "pointer",
  display: "inline-flex",
  flexShrink: 0,
} as const;
