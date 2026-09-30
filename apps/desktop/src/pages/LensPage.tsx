import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Page, pageH1Style } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../tauri-bridge/index.js";
import {
  addDashboardItem,
  createDashboard,
  listDashboards,
  runLens,
  type Dashboard,
  type LensRun,
  type SqlCell,
} from "../api.js";
import { showToast } from "../components/toastStore.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { customDashboardRef, lensRef } from "../tabs/pageRefs.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { formatContextMention } from "../agent-context-ref.js";
import { changedParams, parseParamInput } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import { LensResultView } from "../lens/LensResultView.js";

export interface LensPageProps {
  /** `<extension>/<slug>`. */
  lensId: string;
  /** Param values the tab opened with (from its id), e.g. a slot's
   *  `{ effort_id }`; the params form edits on top of them. */
  initialParams?: Record<string, SqlCell>;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}

/**
 * A lens: a user/agent-built query over the semantic layer, read from
 * `oxplow/extensions/<ext>/lenses/<slug>.yaml` in this stream's worktree.
 * Re-runs when oxplow data changes, so it stays live. Params are edited
 * in the right rail (Enter applies). See `.context/extensions.md`.
 */
export function LensPage({ lensId, initialParams, stream, onOpenPage }: LensPageProps) {
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
  const title = lens?.title ?? lensId;
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
      <PinToDashboard lensId={lensId} onOpenPage={onOpenPage} />
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
function PinToDashboard({ lensId, onOpenPage }: { lensId: string; onOpenPage(ref: TabRef): void }) {
  const [dashboards, setDashboards] = useState<Dashboard[] | null>(null);
  const wrapRef = useRef<HTMLSpanElement | null>(null);
  const isOpen = dashboards !== null;

  // Close on an outside press or Escape, like the app's other popovers.
  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: PointerEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setDashboards(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDashboards(null);
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);

  async function pin(dashboardId: string, title: string) {
    setDashboards(null);
    try {
      await addDashboardItem({
        dashboardId,
        kind: "lens",
        optionsJson: JSON.stringify({ lensId, size: "wide" }),
      });
      showToast({ message: `Pinned to ${title}.` });
      onOpenPage(customDashboardRef(dashboardId));
    } catch (e) {
      recordOpError({ label: "Pin lens to dashboard", message: String(e) });
    }
  }

  async function toggle() {
    if (dashboards) {
      setDashboards(null);
      return;
    }
    try {
      setDashboards(await listDashboards());
    } catch (e) {
      recordOpError({ label: "List dashboards", message: String(e) });
    }
  }

  async function pinToNew() {
    try {
      const d = await createDashboard("My Dashboard");
      await pin(d.id, d.title);
    } catch (e) {
      recordOpError({ label: "Create dashboard", message: String(e) });
    }
  }

  return (
    <span ref={wrapRef} style={{ position: "relative" }}>
      <button type="button" data-testid="lens-pin" onClick={() => void toggle()}>
        Pin to Dashboard
      </button>
      {dashboards ? (
        <div data-testid="lens-pin-menu" style={pinMenuStyle}>
          {dashboards.map((d) => (
            <button
              key={d.id}
              type="button"
              data-testid={`lens-pin-to-${d.id}`}
              style={pinItemStyle}
              onClick={() => void pin(d.id, d.title)}
            >
              {d.title}
            </button>
          ))}
          <button type="button" data-testid="lens-pin-new" style={pinItemStyle} onClick={() => void pinToNew()}>
            New Dashboard…
          </button>
        </div>
      ) : null}
    </span>
  );
}

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

const pinMenuStyle: CSSProperties = {
  position: "absolute",
  top: "100%",
  right: 0,
  zIndex: 10,
  marginTop: 4,
  minWidth: 200,
  display: "flex",
  flexDirection: "column",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 4,
};
const pinItemStyle: CSSProperties = {
  background: "none",
  border: "none",
  textAlign: "left",
  padding: "6px 8px",
  font: "inherit",
  color: "var(--text-primary)",
  cursor: "pointer",
};
const errorStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  margin: "12px 0",
  background: "var(--surface-card)",
};
