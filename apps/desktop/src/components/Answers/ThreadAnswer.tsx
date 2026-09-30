/// One answer an agent showed in a thread (P6.C2): the lens it showed,
/// re-run live, with Keep This (`lens.keep`) — or, once it is a lens, a
/// link to it. Rendered by the Answers strip and inline in an ACP
/// transcript. See `.context/extensions.md` → "Thread answers".
import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import { runAnswer, type LensRun } from "../../api.js";
import { LensResultView } from "../../lens/LensResultView.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { RouteLink } from "../../tabs/RouteLink.js";
import { lensRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { keepAnswer, type AnswerRow } from "../../threadAnswers.js";
import { recordOpError } from "../opErrorsStore.js";

export interface ThreadAnswerProps {
  answer: AnswerRow;
  /** Where links go; without it they navigate through the route context. */
  onOpenPage?(ref: TabRef): void;
}

export function ThreadAnswer({ answer, onOpenPage }: ThreadAnswerProps) {
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [kept, setKept] = useState<string | null>(answer.keptLens);
  useEffect(() => setKept(answer.keptLens), [answer.keptLens]);
  const refresh = useCallback(async () => {
    try {
      setRun(await runAnswer(answer.ref));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [answer.ref]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(run?.result.reads ?? NO_READS, () => void refresh());

  const lens = kept ?? answer.lens;
  return (
    <div data-testid="thread-answer" style={cardStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
        <span data-testid="thread-answer-title" style={{ fontWeight: 600, flex: 1 }}>
          {answer.title}
        </span>
        {lens ? (
          <RouteLink
            to={lensRef(lens)}
            onNavigate={onOpenPage ? () => onOpenPage(lensRef(lens)) : undefined}
            testId="thread-answer-lens"
            title="Open the lens"
            style={linkStyle}
          >
            {lens}
          </RouteLink>
        ) : (
          <KeepThis answer={answer.ref} onKept={setKept} />
        )}
      </div>
      {error ? (
        <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)" }}>{error}</div>
      ) : run ? (
        <LensResultView run={run} toolbar={false} maxRows={20} onOpenPage={onOpenPage} />
      ) : (
        <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>Loading…</div>
      )}
    </div>
  );
}

/** Keep This: an inline slug (empty takes one from the title); Enter
 *  keeps, Escape cancels. */
function KeepThis({ answer, onKept }: { answer: string; onKept(lens: string): void }) {
  const [editing, setEditing] = useState(false);
  const [slug, setSlug] = useState("");
  const [busy, setBusy] = useState(false);
  if (!editing) {
    return (
      <button type="button" data-testid="thread-answer-keep" onClick={() => setEditing(true)}>
        Keep This
      </button>
    );
  }
  return (
    <form
      style={{ display: "flex", gap: 4 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (busy) return;
        setBusy(true);
        keepAnswer(answer, slug)
          .then((lens) => {
            setEditing(false);
            onKept(lens);
          })
          .catch((err: unknown) => recordOpError({ label: "Keep This", message: err instanceof Error ? err.message : String(err) }))
          .finally(() => setBusy(false));
      }}
    >
      <input
        data-testid="thread-answer-slug"
        autoFocus
        placeholder="Name (from its title)"
        value={slug}
        disabled={busy}
        onChange={(e) => setSlug(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            setEditing(false);
            setSlug("");
          }
        }}
      />
      <button type="submit" data-testid="thread-answer-keep-submit" disabled={busy}>
        {busy ? "Keeping…" : "Keep"}
      </button>
    </form>
  );
}

const cardStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 8,
  background: "var(--surface-card)",
};
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
};
