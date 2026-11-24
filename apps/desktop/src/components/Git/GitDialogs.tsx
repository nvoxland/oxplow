/// Git ▸'s dialogs (`gitDialogs.ts`), drawn once by the window in its tab
/// area: Compare with… picks a branch, a tag or a commit to diff a file
/// against; Rollback… picks which of the stream's changes to throw away.
/// Each is a `TabModal`: Enter does it, Escape cancels.
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { querySql, runCommand } from "../../api.js";
import type { GitStatusEntry } from "../../gitMenu.js";
import { gitRevision, WORKING } from "../../revision.js";
import { diffRef } from "../../tabs/pageRefs.js";
import { TabModal } from "../TabModal.js";
import { closeGitDialog, useGitDialog } from "./gitDialogStore.js";
import { compareChoices, rollbackRows, type CompareData, type RollbackRow } from "./gitDialogModel.js";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The open dialog, if any. `openRef` opens a ref in a tab. */
export function GitDialogs({ streamId, openRef }: { streamId: string; openRef(ref: string): void }) {
  const open = useGitDialog();
  if (!open) return null;
  if (open.kind === "compare") {
    return (
      <CompareWithDialog
        key={open.path}
        streamId={streamId}
        path={open.path}
        onPick={(rev) => {
          closeGitDialog();
          openRef(diffRef({ path: open.path, leftVersion: gitRevision(rev), rightVersion: WORKING, baseLabel: rev }).id);
        }}
        onClose={closeGitDialog}
      />
    );
  }
  return (
    <RollbackDialog
      streamId={streamId}
      paths={open.paths}
      checked={open.checked}
      openRef={openRef}
      onClose={closeGitDialog}
    />
  );
}

/** Compare with…: one search over the branches, the tags and the file's
 *  recent commits (`compareChoices`); arrows move, Enter picks. */
export function CompareWithDialog({
  streamId,
  path,
  onPick,
  onClose,
}: {
  streamId: string;
  path: string;
  onPick(rev: string): void;
  onClose(): void;
}) {
  const [query, setQuery] = useState("");
  const [data, setData] = useState<CompareData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [at, setAt] = useState(0);
  useEffect(() => {
    let live = true;
    void Promise.all([
      runCommand("oxplow.vcs.branches", { stream_id: streamId }),
      querySql("SELECT name FROM v_tag ORDER BY name DESC"),
      runCommand("oxplow.vcs.log", { stream_id: streamId, path, limit: 30 }),
    ])
      .then(([branches, tags, log]) => {
        if (!live) return;
        setData({
          branches: (branches.result as CompareData["branches"] | null) ?? [],
          tags: tags.rows.map((r) => String(r[0])),
          commits: ((log.result as { revisions?: CompareData["commits"] } | null)?.revisions ?? []),
        });
      })
      .catch((e: unknown) => live && setError(message(e)));
    return () => {
      live = false;
    };
  }, [streamId, path]);
  const choices = useMemo(() => (data ? compareChoices(query, data) : []), [data, query]);
  const current = Math.min(at, Math.max(0, choices.length - 1));
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${current}"]`)?.scrollIntoView({ block: "nearest" });
  }, [current]);
  const pick = (i: number) => {
    const c = choices[i];
    if (c) onPick(c.rev);
  };
  return (
    <TabModal title="Compare with…" subtitle={path} onClose={onClose} testId="git-compare-dialog">
      <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: "0 16px 16px" }}>
        <input
          className="ox-input"
          data-testid="git-compare-search"
          autoFocus
          placeholder="A branch, tag, commit or revision"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setAt(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setAt(Math.min(current + 1, choices.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setAt(Math.max(current - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              pick(current);
            }
          }}
        />
        {error ? <p className="ox-error">{error}</p> : null}
        {!data && !error ? <p className="ox-empty-line">Reading the branches, tags and history…</p> : null}
        {data && choices.length === 0 ? <p className="ox-empty-line">Nothing to compare with.</p> : null}
        <div ref={list} style={listStyle} role="listbox" data-testid="git-compare-choices">
          {choices.map((c, i) => (
            <div key={`${c.section}:${c.rev}`}>
              {i === 0 || choices[i - 1]!.section !== c.section ? <div style={sectionStyle}>{c.section}</div> : null}
              <button
                type="button"
                role="option"
                aria-selected={i === current}
                data-index={i}
                data-testid={`git-compare-choice-${c.rev}`}
                className="oxplow-menu-item"
                data-open={i === current ? "true" : undefined}
                style={choiceStyle}
                onMouseEnter={() => setAt(i)}
                onClick={() => pick(i)}
              >
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.label}</span>
                {c.detail ? <span style={detailStyle}>{c.detail}</span> : null}
              </button>
            </div>
          ))}
        </div>
      </div>
    </TabModal>
  );
}

/** Rollback…: the stream's changes as a checklist (`rollbackRows`), the
 *  asked ones checked; a name opens its diff. Rollback throws the checked
 *  ones away (`vcs.discard`): this dialog is its confirmation. */
export function RollbackDialog({
  streamId,
  paths,
  checked,
  openRef,
  onClose,
}: {
  streamId: string;
  paths: string[];
  checked: string[];
  openRef(ref: string): void;
  onClose(): void;
}) {
  const [rows, setRows] = useState<RollbackRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  useEffect(() => {
    let live = true;
    void runCommand("oxplow.vcs.status", { stream_id: streamId })
      .then(({ result }) => {
        if (!live) return;
        const entries = ((result as { entries?: GitStatusEntry[] } | null)?.entries ?? []).filter((e) =>
          paths.includes(e.path.replace(/\/+$/, "")),
        );
        setRows(rollbackRows(entries, checked));
      })
      .catch((e: unknown) => live && setError(message(e)));
    return () => {
      live = false;
    };
    // The asked paths are the dialog's; a new ask is a new dialog.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamId]);
  const chosen = (rows ?? []).filter((r) => r.checked);
  const setAll = (on: boolean) => setRows((rs) => rs?.map((r) => ({ ...r, checked: on })) ?? rs);
  const rollback = () => {
    if (chosen.length === 0 || running) return;
    setRunning(true);
    setError(null);
    void runCommand("oxplow.vcs.discard", { stream: streamId, paths: chosen.map((r) => r.path) }, true)
      .then(() => onClose())
      .catch((e: unknown) => {
        setError(message(e));
        setRunning(false);
      });
  };
  return (
    <TabModal title="Rollback…" subtitle="Throw away the changes of the files checked" onClose={onClose} testId="git-rollback-dialog">
      <div
        style={{ display: "flex", flexDirection: "column", gap: 8, padding: "0 16px 16px" }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) {
            e.preventDefault();
            rollback();
          }
        }}
      >
        {!rows && !error ? <p className="ox-empty-line">Reading the changes…</p> : null}
        {rows && rows.length === 0 ? <p className="ox-empty-line">Nothing is changed.</p> : null}
        {rows && rows.length > 0 ? (
          <>
            <div style={{ display: "flex", gap: 8 }}>
              <button type="button" className="ox-button" data-testid="git-rollback-all" onClick={() => setAll(true)}>
                Select All
              </button>
              <button type="button" className="ox-button" data-testid="git-rollback-none" onClick={() => setAll(false)}>
                Select None
              </button>
            </div>
            <div style={listStyle} data-testid="git-rollback-files">
              {rows.map((r, i) => (
                <div key={r.path}>
                  {i === 0 || rows[i - 1]!.folder !== r.folder ? <div style={sectionStyle}>{r.folder || "/"}</div> : null}
                  <label style={rowStyle}>
                    <input
                      type="checkbox"
                      data-testid={`git-rollback-check-${r.path}`}
                      checked={r.checked}
                      autoFocus={i === 0}
                      onChange={(e) => setRows((rs) => rs?.map((x) => (x.path === r.path ? { ...x, checked: e.target.checked } : x)) ?? rs)}
                    />
                    <span style={statusStyle} title={r.status}>
                      {STATUS_MARK[r.status]}
                    </span>
                    <button
                      type="button"
                      style={nameStyle}
                      title={`Show the diff of ${r.path}`}
                      disabled={r.status === "untracked"}
                      onClick={() =>
                        openRef(diffRef({ path: r.path, leftVersion: gitRevision("HEAD"), rightVersion: WORKING, baseLabel: "HEAD" }).id)
                      }
                    >
                      {r.name}
                    </button>
                    {r.deletes ? <span style={detailStyle}>deleted: it is untracked</span> : null}
                  </label>
                </div>
              ))}
            </div>
          </>
        ) : null}
        {error ? <p className="ox-error" data-testid="git-rollback-error">{error}</p> : null}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" className="ox-button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="ox-button ox-primary"
            data-testid="git-rollback-run"
            disabled={chosen.length === 0 || running}
            onClick={rollback}
          >
            {running ? "Rolling back…" : chosen.length === 1 ? "Rollback 1 File" : `Rollback ${chosen.length} Files`}
          </button>
        </div>
      </div>
    </TabModal>
  );
}

const STATUS_MARK: Record<GitStatusEntry["status"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "C",
};

const listStyle: CSSProperties = { maxHeight: "50vh", overflowY: "auto", display: "flex", flexDirection: "column", gap: 2 };
const sectionStyle: CSSProperties = {
  fontSize: 11,
  color: "var(--muted)",
  padding: "6px 8px 2px",
  textTransform: "uppercase",
  letterSpacing: 0.4,
};
const choiceStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  border: "none",
  borderRadius: 4,
  padding: "6px 8px",
  color: "var(--fg)",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
  textAlign: "left",
  cursor: "pointer",
};
const detailStyle: CSSProperties = { color: "var(--muted)", fontSize: 11, flexShrink: 0 };
const rowStyle: CSSProperties = { display: "flex", alignItems: "center", gap: 8, padding: "3px 8px", fontSize: "var(--text-xs)" };
const statusStyle: CSSProperties = { width: 12, color: "var(--muted)", fontFamily: "var(--font-mono)" };
const nameStyle: CSSProperties = {
  border: "none",
  background: "transparent",
  color: "var(--fg)",
  padding: 0,
  font: "inherit",
  cursor: "pointer",
  textAlign: "left",
};
