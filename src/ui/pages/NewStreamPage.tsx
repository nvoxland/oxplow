import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import {
  createStream,
  getDefaultBranch,
  listAdoptableWorktrees,
  listBranches,
  type BranchRef,
  type GitWorktreeEntry,
  type Stream,
} from "../api.js";
import { logUi } from "../logger.js";
import { BranchPicker, type PickedRef } from "../components/BranchPicker.js";
import { Page } from "../tabs/Page.js";

export type NewStreamMode = "existing" | "new" | "worktree";

export interface NewStreamFormState {
  mode: NewStreamMode;
  title: string;
  selectedRef: string;
  newBranch: string;
  startPointRef: string;
  worktreePath: string;
}

/**
 * Pure validator for the New stream form. Centralises the per-mode
 * required-field rules so they can be tested without mounting the page.
 */
export function validateNewStreamInput(state: NewStreamFormState):
  | { ok: true }
  | { ok: false; message: string } {
  if (!state.title.trim()) return { ok: false, message: "Name is required" };
  if (state.mode === "existing" && !state.selectedRef) {
    return { ok: false, message: "Select an existing branch" };
  }
  if (state.mode === "new") {
    if (!state.newBranch.trim()) return { ok: false, message: "Enter a new branch name" };
    if (!state.startPointRef) return { ok: false, message: "Choose a starting branch" };
  }
  if (state.mode === "worktree" && !state.worktreePath) {
    return { ok: false, message: "Select a worktree" };
  }
  return { ok: true };
}

/**
 * Pick the branch entry that represents the repo's default branch
 * (e.g. main / master). `defaultBranch` is the short name returned by
 * `detectBaseBranch` and may be either local ("main") or remote-qualified
 * ("origin/main"). We prefer a matching local branch, then fall back to the
 * remote-tracking branch.
 */
export function pickDefaultBranchEntry(
  branches: BranchRef[],
  defaultBranch: string | null,
): BranchRef | null {
  if (!defaultBranch) return null;
  const stripped = defaultBranch.replace(/^[^/]+\//, "");
  const localMatch = branches.find((b) => b.kind === "local" && b.name === stripped);
  if (localMatch) return localMatch;
  const remoteMatch = branches.find((b) => b.kind === "remote" && b.name === defaultBranch);
  if (remoteMatch) return remoteMatch;
  const remoteByShort = branches.find(
    (b) => b.kind === "remote" && b.name.replace(/^[^/]+\//, "") === stripped,
  );
  return remoteByShort ?? null;
}

function resolvePickedRef(target: PickedRef): { ref: string; label: string } {
  if (target.kind === "tag") {
    return { ref: `refs/tags/${target.name}`, label: `tag: ${target.name}` };
  }
  const branch = target.branch;
  if (!branch) return { ref: "", label: target.name };
  return { ref: branch.ref, label: `[${branch.kind}] ${branch.name}` };
}

export interface NewStreamPageProps {
  /** Whether git operations are available in this workspace. */
  gitEnabled: boolean;
  /** Existing streams count, for the default "Stream N" placeholder. */
  defaultTitle?: string;
  onClose?(): void;
  /** Fired after a successful create. The host adds the new stream to its list and closes the page. */
  onCreated(stream: Stream): void;
}

/**
 * Full-tab "New stream" form. Replaces the inline modal that used to
 * live inside `StreamRail.tsx`. Form layout matches the legacy modal
 * (existing branch / new branch / existing worktree) so muscle memory
 * carries over.
 */
export function NewStreamPage({ gitEnabled, defaultTitle, onClose, onCreated }: NewStreamPageProps) {
  const [mode, setMode] = useState<NewStreamMode>("existing");
  const [title, setTitle] = useState(defaultTitle ?? "");
  const [summary, setSummary] = useState("");
  const [selectedRef, setSelectedRef] = useState("");
  const [selectedRefLabel, setSelectedRefLabel] = useState("");
  const [newBranch, setNewBranch] = useState("");
  const [startPointRef, setStartPointRef] = useState("");
  const [startPointLabel, setStartPointLabel] = useState("");
  const [worktreePath, setWorktreePath] = useState("");
  const [worktrees, setWorktrees] = useState<GitWorktreeEntry[]>([]);
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [creating, setCreating] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const titleRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!gitEnabled) return;
    let cancelled = false;
    setLoadingBranches(true);
    void Promise.all([listBranches(), listAdoptableWorktrees(), getDefaultBranch()])
      .then(([nextBranches, nextWorktrees, defaultBranch]) => {
        if (cancelled) return;
        setWorktrees(nextWorktrees);
        const first = nextBranches[0];
        if (first) {
          setSelectedRef((prev) => prev || first.ref);
          setSelectedRefLabel((prev) => prev || first.name);
        }
        const defaultEntry = pickDefaultBranchEntry(nextBranches, defaultBranch) ?? first;
        if (defaultEntry) {
          setStartPointRef((prev) => prev || defaultEntry.ref);
          setStartPointLabel((prev) => prev || defaultEntry.name);
        }
        const firstWt = nextWorktrees[0];
        if (firstWt) setWorktreePath((prev) => prev || firstWt.path);
        logUi("info", "loaded branch list", {
          branchCount: nextBranches.length,
          worktreeCount: nextWorktrees.length,
        });
      })
      .catch((err) => {
        if (cancelled) return;
        setFormError(String(err));
        logUi("error", "failed to load branch list", { error: String(err) });
      })
      .finally(() => {
        if (!cancelled) setLoadingBranches(false);
      });
    return () => {
      cancelled = true;
    };
  }, [gitEnabled]);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  async function handleCreate() {
    const validation = validateNewStreamInput({
      mode,
      title,
      selectedRef,
      newBranch,
      startPointRef,
      worktreePath,
    });
    if (!validation.ok) {
      setFormError(validation.message);
      return;
    }
    setCreating(true);
    setFormError(null);
    try {
      const created =
        mode === "existing"
          ? await createStream({
              title: title.trim(),
              summary: summary.trim(),
              source: "existing",
              ref: selectedRef,
            })
          : mode === "new"
            ? await createStream({
                title: title.trim(),
                summary: summary.trim(),
                source: "new",
                branch: newBranch.trim(),
                startPointRef,
              })
            : await createStream({
                title: title.trim(),
                summary: summary.trim(),
                source: "worktree",
                worktreePath,
              });
      onCreated(created);
      onClose?.();
    } catch (err) {
      setFormError(String(err));
    } finally {
      setCreating(false);
    }
  }

  if (!gitEnabled) {
    return (
      <Page testId="page-new-stream" title="New stream" kind="new stream">
        <div style={{ padding: "20px 24px", color: "var(--text-secondary)", fontSize: 13 }}>
          This workspace root does not contain its own <code>.git</code> directory, so streams
          cannot be created here. Open a git-enabled workspace to add streams.
        </div>
      </Page>
    );
  }

  return (
    <Page
      testId="page-new-stream"
      title="New stream"
      kind="new stream"
      actions={
        onClose ? (
          <button type="button" onClick={onClose} style={buttonStyle}>
            Close
          </button>
        ) : null
      }
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void handleCreate();
        }}
        style={{ padding: "20px 24px", maxWidth: 720, display: "flex", flexDirection: "column", gap: 14 }}
      >
        <Field label="Name">
          <input
            ref={titleRef}
            data-testid="new-stream-title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            style={inputStyle}
          />
        </Field>
        <Field label="Summary">
          <input
            data-testid="new-stream-summary"
            value={summary}
            onChange={(e) => setSummary(e.target.value)}
            style={inputStyle}
          />
        </Field>
        <Field label="Branch source">
          <select
            data-testid="new-stream-mode"
            value={mode}
            onChange={(e) => setMode(e.target.value as NewStreamMode)}
            style={inputStyle}
          >
            <option value="existing">Existing branch</option>
            <option value="new">Create new branch</option>
            <option value="worktree" disabled={worktrees.length === 0}>
              {worktrees.length === 0 ? "Existing worktree (none available)" : "Existing worktree"}
            </option>
          </select>
        </Field>
        {mode === "existing" ? (
          <Field label="Existing branch">
            <BranchPicker
              label={<span>{selectedRefLabel || "Select branch…"}</span>}
              anchor="bottom"
              align="left"
              currentBranch={null}
              disabled={loadingBranches}
              buttonStyle={pickerButtonStyle}
              onPick={(target) => {
                const { ref, label } = resolvePickedRef(target);
                setSelectedRef(ref);
                setSelectedRefLabel(label);
              }}
            />
          </Field>
        ) : mode === "new" ? (
          <>
            <Field label="New branch">
              <input
                data-testid="new-stream-branch"
                value={newBranch}
                onChange={(e) => setNewBranch(e.target.value)}
                style={inputStyle}
              />
            </Field>
            <Field label="Start point">
              <BranchPicker
                label={<span>{startPointLabel || "Select starting ref…"}</span>}
                anchor="bottom"
                align="left"
                currentBranch={null}
                disabled={loadingBranches}
                buttonStyle={pickerButtonStyle}
                onPick={(target) => {
                  const { ref, label } = resolvePickedRef(target);
                  setStartPointRef(ref);
                  setStartPointLabel(label);
                }}
              />
            </Field>
          </>
        ) : (
          <Field label="Existing worktree">
            <select
              data-testid="new-stream-worktree"
              value={worktreePath}
              onChange={(e) => setWorktreePath(e.target.value)}
              style={inputStyle}
              disabled={loadingBranches}
            >
              {worktrees.map((wt) => (
                <option key={wt.path} value={wt.path}>
                  {wt.branch ? `[${wt.branch}]` : "[detached]"} {wt.path}
                </option>
              ))}
            </select>
          </Field>
        )}

        <div style={{ color: formError ? "var(--severity-critical)" : "var(--text-secondary)", fontSize: 12 }}>
          {formError ?? "Each stream gets its own worktree and Claude resume metadata."}
        </div>

        <div style={actionsRowStyle}>
          <span style={{ flex: 1 }} />
          <button type="button" onClick={onClose} style={buttonStyle}>
            Cancel
          </button>
          <button
            type="submit"
            data-testid="new-stream-create"
            style={primaryButtonStyle}
            disabled={creating || loadingBranches}
          >
            {creating ? "Creating…" : "Create stream"}
          </button>
        </div>
      </form>
    </Page>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
      <span style={{ color: "var(--text-secondary)", fontWeight: 500 }}>{label}</span>
      {children}
    </label>
  );
}

const inputStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "6px 10px",
  fontFamily: "inherit",
  fontSize: 13,
};

const pickerButtonStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "6px 10px",
  borderRadius: 6,
  fontFamily: "inherit",
  fontSize: 13,
  minWidth: 220,
  justifyContent: "flex-start",
  height: "auto",
};

const buttonStyle: CSSProperties = {
  background: "var(--surface-tab-inactive)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "6px 14px",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 13,
};

const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  background: "var(--accent)",
  borderColor: "var(--accent)",
  color: "var(--accent-on-accent)",
};

const actionsRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  paddingTop: 12,
  borderTop: "1px solid var(--border-subtle)",
};
