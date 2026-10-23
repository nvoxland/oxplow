/// `answer` viz (`.context/extensions.md` "Thread answers"): what an agent
/// showed with `oxplow_show_lens` — an existing lens with its params, or
/// its own spec — re-run live, with **Keep This** (`oxplow.lens.keep`: an
/// inline name, empty takes it from the title; Enter keeps, Escape
/// cancels) until it is a lens, and then a link to that lens. Foundation's
/// `answer` page draws an `oxplow/answer` URI with it.
import { useCallback, useEffect, useState, type CSSProperties } from "react";

import { keptLens, runAnswer, runCommand, type KeptLens, type LensRun } from "../api.js";
import { recordOpError } from "../opErrors.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import { idOf } from "../uri/uri.js";
import { LensResultView } from "./LensResultView.js";
import { NO_READS, useRerunOnChange } from "./lensRerun.js";

/** Keep This: the answer becomes a private lens (`oxplow.lens.keep`) in its
 *  thread's worktree; an empty slug lets the command take one from the
 *  title. */
export async function keepAnswer(answer: string, slug: string): Promise<KeptLens> {
  const id = idOf(answer, "oxplow/answer");
  if (id === null) throw new Error(`not an answer: ${answer}`);
  const input: Record<string, string | number> = { answer: Number(id) };
  if (slug.trim() !== "") input.slug = slug.trim();
  const outcome = await runCommand("oxplow.lens.keep", input);
  return keptLens(outcome.result);
}

export function AnswerViz({ uri, kept, onOpenPage }: { uri: string | null; kept: string | null; onOpenPage?(ref: TabRef): void }) {
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [keptNow, setKept] = useState<string | null>(kept);
  useEffect(() => setKept(kept), [kept]);
  const refresh = useCallback(async () => {
    if (!uri) return;
    try {
      setRun(await runAnswer(uri));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [uri]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(run?.result.reads ?? NO_READS, () => void refresh());

  if (!uri) return <p className="ox-empty-line">This page names no answer.</p>;
  // The lens it is: the one it was kept as, else the existing lens it
  // showed (an answer's own spec runs as `answer/<n>`).
  const lens = keptNow ?? (run && !run.lens.id.startsWith("answer/") ? run.lens.id : null);
  return (
    <div data-testid="lens-answer">
      <div style={barStyle}>
        {lens ? (
          <RouteLink
            to={lensRef(lens)}
            onNavigate={onOpenPage ? () => onOpenPage(lensRef(lens)) : undefined}
            testId="lens-answer-lens"
            title="Open the lens"
            style={linkStyle}
          >
            {keptNow ? `Kept as ${lens}` : `Shows ${lens}`}
          </RouteLink>
        ) : run ? (
          <KeepThis answer={uri} onKept={setKept} />
        ) : null}
      </div>
      {error ? (
        <p className="ox-error" data-testid="lens-answer-error">{error}</p>
      ) : run ? (
        <div data-testid="lens-answer-body">
          <LensResultView run={run} toolbar={false} onOpenPage={onOpenPage} />
        </div>
      ) : (
        <p className="ox-empty-line">Loading…</p>
      )}
    </div>
  );
}

function KeepThis({ answer, onKept }: { answer: string; onKept(lens: string): void }) {
  const [editing, setEditing] = useState(false);
  const [slug, setSlug] = useState("");
  const [busy, setBusy] = useState(false);
  if (!editing) {
    return (
      <button type="button" data-testid="lens-answer-keep" onClick={() => setEditing(true)}>
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
          .then((done) => {
            setEditing(false);
            onKept(done.lens);
          })
          .catch((err: unknown) => recordOpError({ label: "Keep This", message: err instanceof Error ? err.message : String(err) }))
          .finally(() => setBusy(false));
      }}
    >
      <input
        data-testid="lens-answer-slug"
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
      <button type="submit" data-testid="lens-answer-keep-submit" disabled={busy}>
        {busy ? "Keeping…" : "Keep"}
      </button>
    </form>
  );
}

const barStyle: CSSProperties = { display: "flex", alignItems: "center", gap: 8, marginBottom: 8 };
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
};
