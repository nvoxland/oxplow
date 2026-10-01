import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Page, pageH1Style } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../tauri-bridge/index.js";
import { runLens, type LensRun, type SqlCell } from "../api.js";
import { lensRef } from "../tabs/pageRefs.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { formatContextMention } from "../agent-context-ref.js";
import { changedParams, parseParamInput } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import { LensResultView } from "../lens/LensResultView.js";
import { PinToDashboard } from "../components/Dashboard/PinToDashboard.js";

export interface LensPageProps {
  /** `<extension>/<slug>`. */
  lensId: string;
  /** Param values the tab opened with (from its id), e.g. a slot's
   *  `{ effort_id }`; the params form edits on top of them. */
  initialParams?: Record<string, SqlCell>;
  /** The page's name as its host knows it (an extension page's manifest
   *  title); the lens's own title otherwise. */
  title?: string;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}

/**
 * A lens: a user/agent-built query over the semantic layer, read from
 * `oxplow/extensions/<ext>/lenses/<slug>.yaml` in this stream's worktree.
 * Re-runs when oxplow data changes, so it stays live. Params are edited
 * in the right rail (Enter applies). See `.context/extensions.md`.
 */
export function LensPage({ lensId, initialParams, title: pageTitle, stream, onOpenPage }: LensPageProps) {
  const streamId = stream?.id ?? null;
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [overrides, setOverrides] = useState<Record<string, SqlCell>>(initialParams ?? {});
  const overridesRef = useRef(overrides);
  overridesRef.current = overrides;

  const guard = useRequestGuard();

  const refresh = useCallback(async () => {
    const current = guard.begin();
    try {
      const next = await runLens(lensId, overridesRef.current, streamId);
      if (!current()) return;
      setRun(next);
      setError(null);
    } catch (e) {
      if (current()) setError(e instanceof Error ? e.message : String(e));
    }
  }, [lensId, streamId, guard]);

  // Another lens or stream: its old result mustn't show under the new one.
  useEffect(() => {
    setRun(null);
    setError(null);
  }, [lensId, streamId]);

  useEffect(() => {
    void refresh();
  }, [refresh, overrides]);

  useRerunOnChange(run?.result.reads ?? NO_READS, () => void refresh());

  const lens = run?.lens ?? null;
  const title = pageTitle ?? lens?.title ?? lensId;
  usePageTitle(title);

  // Publish this lens's live params so an agent asking "what am I looking
  // at" (`get_open_page`) re-runs it with exactly these values.
  const runParams = run?.params;
  useEffect(() => {
    const store = getPageDetailStore();
    const pageId = lensRef(lensId, initialParams).id;
    store.publish(pageId, { lensId, params: runParams ?? {} });
    return () => store.publish(pageId, null);
  }, [lensId, initialParams, runParams]);

  const improveWithAgent = () => {
    const params = lens ? changedParams(lens, run?.params ?? {}) : {};
    insertIntoAgent(formatContextMention({ kind: "lens", lensId, params }));
  };

  const actions = (
    <div style={{ display: "flex", gap: 8 }}>
      <button type="button" data-testid="lens-refresh" onClick={() => void refresh()}>
        Refresh
      </button>
      <PinToDashboard
        tile={{ kind: "lens", lensId, optionsJson: JSON.stringify({ size: "wide" }) }}
        testId="lens-pin"
        onOpenPage={onOpenPage}
      />
      <button
        type="button"
        data-testid="lens-improve-with-agent"
        title="Add this lens to the agent's context so you can ask it to change the lens"
        onClick={improveWithAgent}
      >
        Improve with Agent
      </button>
    </div>
  );

  const rightRail =
    lens && lens.params.length > 0 ? (
      <ParamsForm
        lens={lens}
        values={run?.params ?? {}}
        onApply={(name, value) => setOverrides((o) => ({ ...o, [name]: value }))}
      />
    ) : undefined;

  return (
    <Page
      testId="page-lens"
      kind="lens"
      titleInBody
      layout="details"
      actions={actions}
      rightRail={rightRail}
      rightRailTitle="Parameters"
    >
      <h1 style={pageH1Style}>{title}</h1>
      {lens?.description ? (
        <p style={{ color: "var(--text-secondary)", marginTop: 0 }}>{lens.description}</p>
      ) : null}
      {error ? (
        <div data-testid="lens-error" style={errorStyle}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>This lens couldn't run.</div>
          <div style={{ fontFamily: "var(--font-mono, monospace)", whiteSpace: "pre-wrap" }}>{error}</div>
          <div style={{ marginTop: 8, color: "var(--text-secondary)" }}>
            Use “Improve with Agent” and ask the agent to fix it; it can check its work with
            <code> validate_extension</code>.
          </div>
        </div>
      ) : null}
      {run ? <LensResultView run={run} onOpenPage={onOpenPage} /> : error ? null : <p>Loading…</p>}
      {lens ? (
        <p style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)", marginTop: 24 }}>
          {lens.path}
        </p>
      ) : null}
    </Page>
  );
}

/** "Pin to Dashboard": adds a `lens` tile to a chosen dashboard (or a new
 *  "My Dashboard" when there are none), then offers to open it. */
function ParamsForm({
  lens,
  values,
  onApply,
}: {
  lens: LensRun["lens"];
  values: Record<string, SqlCell>;
  onApply(name: string, value: SqlCell): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      {lens.params.map((p) => (
        <ParamInput key={p.name} name={p.name} label={p.label ?? p.name} value={values[p.name] ?? null} onApply={onApply} />
      ))}
      <div style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Enter applies; Escape reverts.</div>
    </div>
  );
}

function ParamInput({
  name,
  label,
  value,
  onApply,
}: {
  name: string;
  label: string;
  value: SqlCell;
  onApply(name: string, value: SqlCell): void;
}) {
  const shown = value === null ? "" : String(value);
  const [text, setText] = useState(shown);
  useEffect(() => setText(shown), [shown]);
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: "var(--text-sm)" }}>
      {label}
      <input
        data-testid={`lens-param-${name}`}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onApply(name, parseParamInput(text));
          if (e.key === "Escape") setText(shown);
        }}
        onBlur={() => {
          if (text !== shown) onApply(name, parseParamInput(text));
        }}
      />
    </label>
  );
}

const errorStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  margin: "12px 0",
  background: "var(--surface-card)",
};
