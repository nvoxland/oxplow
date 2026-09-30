import { LensSlots } from "../lens/LensSlots.js";
import { useChange } from "../lens/useChange.js";
import { useCallback, useState } from "react";
import type { DiffEntry, Stream } from "../api.js";
import { gitCommitAll } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { indexRef, opErrorRef } from "../tabs/pageRefs.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { ChangedFilesTree } from "../components/ChangedFiles/ChangedFilesTree.js";
import { useChangedFiles } from "../components/ChangedFiles/useChangedFiles.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";

export interface UncommittedChangesPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
}

/**
 * Working-tree page: a commit form (commits every changed file), the
 * changed files, and the `uncommitted` lens slot (change analysis comes
 * from extensions there, reading `v_change*` for this `change_id`).
 */
export function UncommittedChangesPage({
  stream,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: UncommittedChangesPageProps) {
  const streamId = stream?.id ?? null;
  const { change } = useChange(streamId ? { kind: "working", streamId } : null);
  const changed = useChangedFiles(streamId ? { kind: "working", streamId } : null);
  const [committing, setCommitting] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");

  const fileCount = changed.files.length;
  const hasUntracked = changed.files.some((f) => f.status === "untracked");

  const onCommit = useCallback(async () => {
    if (!streamId) return;
    const message = commitMessage.trim();
    if (!message) return;
    if (fileCount === 0) return;
    setCommitting(true);
    try {
      const allPaths = changed.files.map((f) => f.path);
      const result = await gitCommitAll(streamId, message, {
        paths: allPaths,
        includeUntracked: hasUntracked,
      });
      if (!result.success) {
        const errorId = recordOpError({
          label: "Commit all changes",
          command: `git commit -am "${message.trim()}"`,
          stderr: result.stderr ?? "",
          stdout: result.stdout ?? "",
          exitCode: result.status ?? null,
        });
        onOpenPage(opErrorRef(errorId), { newTab: true });
      } else {
        setCommitMessage("");
        await changed.refresh();
      }
    } finally {
      setCommitting(false);
    }
  }, [streamId, onOpenPage, commitMessage, fileCount, changed, hasUntracked]);

  const openDiff = (path: string) => {
    if (!changed.base || !changed.head) return onOpenFile(path);
    const spec: DiffSpec = { path, leftVersion: changed.base, rightVersion: changed.head, baseLabel: "HEAD" };
    if (onOpenDiffInTab) onOpenDiffInTab(spec);
    else if (onOpenDiff) onOpenDiff(spec);
    else onOpenFile(path);
  };

  if (!streamId) {
    return (
      <Page testId="page-uncommitted-changes" title="Uncommitted Changes">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-uncommitted-changes" title="Uncommitted Changes">
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {changed.error ? <div style={errorBanner}>{changed.error}</div> : null}

        {fileCount > 0 ? (
          <section data-testid="uncommitted-commit-form" style={card}>
            <div style={{ fontWeight: 600, marginBottom: 8 }}>Commit</div>
            <textarea
              data-testid="uncommitted-commit-message"
              value={commitMessage}
              onChange={(e) => setCommitMessage(e.target.value)}
              placeholder="Commit message"
              rows={3}
              style={{
                width: "100%",
                boxSizing: "border-box",
                padding: 8,
                fontFamily: "inherit",
                fontSize: "var(--text-sm)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 4,
                background: "var(--surface-input, var(--surface-card))",
                color: "var(--text-primary)",
                resize: "vertical",
              }}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                  e.preventDefault();
                  void onCommit();
                }
              }}
            />
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: 8 }}>
              <span style={subtle}>
                {fileCount} file{fileCount === 1 ? "" : "s"} pending
              </span>
              <button
                type="button"
                data-testid="uncommitted-commit-button"
                onClick={onCommit}
                disabled={committing || fileCount === 0 || commitMessage.trim().length === 0}
                style={{
                  ...primaryButton,
                  opacity: committing || fileCount === 0 || commitMessage.trim().length === 0 ? 0.5 : 1,
                  cursor: committing || fileCount === 0 || commitMessage.trim().length === 0 ? "not-allowed" : "pointer",
                }}
              >
                {committing ? "Committing…" : `Commit ${fileCount}`}
              </button>
            </div>
          </section>
        ) : null}

        {fileCount === 0 && !changed.loading ? (
          <div data-testid="uncommitted-clean" style={cleanState}>
            <span>Working tree is clean.</span>
            <button
              type="button"
              onClick={(e) => onOpenPage(indexRef("git-history"), { newTab: e.metaKey || e.ctrlKey })}
              style={historyLink}
            >
              View git history →
            </button>
          </div>
        ) : (
          <>
            <section data-testid="uncommitted-files">
              <div style={{ fontWeight: 600, marginBottom: 8 }}>
                Files <span style={subtle}>{totalsLabel(summarize(changed.files))}</span>
              </div>
              <ChangedFilesTree files={changed.files} onOpenFile={onOpenFile} onOpenFileDiff={openDiff} />
            </section>
            <LensSlots
              slot="uncommitted"
              params={change ? { change_id: change.id } : null}
              streamId={streamId}
              onOpenPage={(ref) => onOpenPage(ref)}
            />
          </>
        )}
      </div>
    </Page>
  );
}

/** Status counts and line totals for a list of changed files. */
export interface SummaryNumbers {
  total: number;
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  additions: number;
  deletions: number;
}

export function summarize(files: DiffEntry[]): SummaryNumbers {
  const out: SummaryNumbers = {
    total: files.length,
    modified: 0,
    added: 0,
    deleted: 0,
    renamed: 0,
    untracked: 0,
    additions: 0,
    deletions: 0,
  };
  for (const file of files) {
    out[file.status] += 1;
    out.additions += file.additions ?? 0;
    out.deletions += file.deletions ?? 0;
  }
  return out;
}

/** `3 changed · +10 −2`: the Files heading's totals. */
export function totalsLabel(n: SummaryNumbers): string {
  return `${n.total} changed · +${n.additions} −${n.deletions}`;
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const cleanState: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 16,
  display: "flex",
  alignItems: "center",
  gap: 12,
  fontSize: "var(--text-sm)",
  color: "var(--text-muted)",
};
const historyLink: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
};
const primaryButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-action, #2563eb)",
  color: "var(--text-inverse, white)",
  border: "none",
  borderRadius: 4,
  cursor: "pointer",
};
