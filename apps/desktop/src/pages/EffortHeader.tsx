import { useCallback, useEffect, useState } from "react";
import type { CSSProperties } from "react";

import { querySql, runCommand } from "../api.js";
import { InlineEdit } from "../components/InlineEdit.js";
import { InlinePromptStrip } from "../components/InlinePromptStrip.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { readsOf, useRerunOnChange } from "../lens/lensRerun.js";
import { effortRowId } from "../lens/lensModel.js";
import { pageH1Style } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { workItemTabRef } from "../tabs/pageRefs.js";
import { workItemLabel } from "../workItemRef.js";
import { useWorkListProfile } from "../useWorkListProfile.js";
import { workItemRefOfMention } from "../workItems.js";
import { effortRef } from "../recordRefs.js";

/** An effort as its page shows it (`v_effort`). */
export interface EffortRecord {
  /** Its title: its own, else its item's, else its first prompt's. */
  title: string | null;
  workItem: string | null;
  endedAt: string | null;
  closedBy: string | null;
}

/** What the header reads and runs — the real API unless a test swaps it. */
export interface EffortHeaderDeps {
  readEffort(row: number): Promise<EffortRecord | null>;
  runCommand(name: string, input: unknown): Promise<unknown>;
}

const text = (c: unknown): string | null => (c === null || c === undefined ? null : String(c));

export const realEffortHeaderDeps: EffortHeaderDeps = {
  async readEffort(row) {
    const result = await querySql(
      "SELECT title, work_item, ended_at, closed_by FROM v_effort WHERE id = ?1",
      [row],
    );
    const r = result.rows[0];
    return r ? { title: text(r[0]), workItem: text(r[1]), endedAt: text(r[2]), closedBy: text(r[3]) } : null;
  },
  runCommand: (name, input) => runCommand(name, input),
};

/** The work-item ref a person typed: an id of the active list's (as it
 *  declares them), or a full `work_item:<provider>:<id>`; null for
 *  anything else. */
export function normalizeWorkItemInput(raw: string, mentionRef: (id: string) => string | null): string | null {
  const v = raw.trim();
  if (/^work_item:[a-z0-9_-]+:\S+$/.test(v)) return v;
  return mentionRef(v);
}

/** How an effort closed, in words. */
export function closedByLabel(closedBy: string | null): string {
  switch (closedBy) {
    case "commit":
      return "Closed by a commit";
    case "switch":
      return "Closed when the thread moved on";
    case "person":
      return "Closed by you";
    case "agent":
      return "Closed by the agent";
    case "system":
      return "Closed by oxplow";
    default:
      return "Closed";
  }
}

/**
 * The effort page's header: its title, editable in place (`oxplow.effort.update`;
 * clearing it restores the default), what it's linked to (`oxplow.effort.link`,
 * from a typed `tsk42` or work-item ref; Unlink clears it), and — while it's
 * open — Close (`oxplow.effort.close`), else how it closed. Re-reads `v_effort`
 * when it changes.
 */
export function EffortHeader({
  effortId,
  onOpenPage,
  onTitle,
  deps = realEffortHeaderDeps,
}: {
  effortId: string;
  onOpenPage(ref: TabRef): void;
  /** The title as read, for the page's tab title. */
  onTitle?(title: string | null): void;
  deps?: EffortHeaderDeps;
}) {
  const profile = useWorkListProfile();
  const row = effortRowId(effortId);
  const ref = effortRef(row === null ? effortId : `eff${row}`);
  const [effort, setEffort] = useState<EffortRecord | null>(null);
  const [linking, setLinking] = useState(false);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    if (row === null) return;
    deps
      .readEffort(row)
      .then((e) => {
        setEffort(e);
        onTitle?.(e?.title ?? null);
      })
      .catch((e: unknown) => recordOpError({ label: "Read the effort", message: String(e) }));
  }, [row, deps, onTitle]);
  useEffect(load, [load]);
  useRerunOnChange(readsOf("v_effort"), load);

  const run = async (label: string, name: string, input: unknown): Promise<boolean> => {
    setBusy(true);
    try {
      await deps.runCommand(name, input);
      load();
      return true;
    } catch (e) {
      recordOpError({ label, command: name, message: e instanceof Error ? e.message : String(e) });
      return false;
    } finally {
      setBusy(false);
    }
  };

  const title = effort?.title ?? "";
  return (
    <div data-testid="effort-header" style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <h1 style={pageH1Style} data-testid="diff-view-title">
        <InlineEdit
          value={title}
          allowEmpty
          placeholder="Untitled effort"
          testId="effort-title"
          ariaLabel="Effort title"
          onCommit={(next) =>
            void run("Rename the effort", "oxplow.effort.update", {
              effort: ref,
              title: next.trim() === "" ? null : next.trim(),
            })
          }
        />
      </h1>
      <div style={lineStyle} data-testid="effort-link-line">
        <span style={labelStyle}>Linked to</span>
        {effort?.workItem ? (
          <>
            <button
              type="button"
              data-testid="effort-linked-item"
              style={linkButtonStyle}
              onClick={() => onOpenPage(workItemTabRef(effort.workItem!))}
            >
              {workItemLabel(effort.workItem)}
            </button>
            <button
              type="button"
              data-testid="effort-unlink"
              style={buttonStyle}
              disabled={busy}
              onClick={() => void run("Unlink the effort", "oxplow.effort.link", { effort: ref, work_item: null })}
            >
              Unlink
            </button>
          </>
        ) : (
          <span style={mutedStyle} data-testid="effort-not-linked">
            Not linked
          </span>
        )}
        {!linking ? (
          <button
            type="button"
            data-testid="effort-link"
            style={buttonStyle}
            disabled={busy}
            onClick={() => setLinking(true)}
          >
            Link to task…
          </button>
        ) : null}
        <span style={{ flex: 1 }} />
        {effort && effort.endedAt === null ? (
          <button
            type="button"
            data-testid="effort-close"
            style={buttonStyle}
            disabled={busy}
            onClick={() => void run("Close the effort", "oxplow.effort.close", { effort: ref })}
          >
            Close effort
          </button>
        ) : effort ? (
          <span style={mutedStyle} data-testid="effort-closed-by">
            {closedByLabel(effort.closedBy)}
          </span>
        ) : null}
      </div>
      {linking ? (
        <InlinePromptStrip
          testId="effort-link-prompt"
          message="Link this effort to a task (tsk42, or a work item ref)."
          fields={[{ key: "item", placeholder: "tsk42" }]}
          confirmLabel="Link"
          busy={busy}
          onCancel={() => setLinking(false)}
          onSubmit={async ({ item }) => {
            const workItem = normalizeWorkItemInput(item ?? "", (id) => workItemRefOfMention(profile, id));
            if (!workItem) {
              recordOpError({ label: "Link the effort", message: `\`${item}\` isn't a work item's id or ref` });
              return;
            }
            if (await run("Link the effort", "oxplow.effort.link", { effort: ref, work_item: workItem })) {
              setLinking(false);
            }
          }}
        />
      ) : null}
    </div>
  );
}

const lineStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontSize: "var(--text-sm)",
};
const labelStyle: CSSProperties = { color: "var(--text-secondary)" };
const mutedStyle: CSSProperties = { color: "var(--text-muted)" };
const linkButtonStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "inherit",
};
const buttonStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "2px 8px",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
  cursor: "pointer",
};
