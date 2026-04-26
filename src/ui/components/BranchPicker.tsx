import type { CSSProperties, MouseEvent as ReactMouseEvent, ReactNode } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { deleteGitBranch, gitMergeInto, gitRebaseOnto, listGitRefs, renameGitBranch, type BranchRef, type GroupedGitRefs } from "../api.js";
import { ContextMenu } from "./ContextMenu.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { Slideover } from "./Slideover.js";

export interface PickedRef {
  kind: "branch" | "tag";
  /** Branch local name (e.g. "main") for kind=branch, or tag name for kind=tag. */
  name: string;
  /** For branches, the original BranchRef (local or remote). Undefined for tags. */
  branch?: BranchRef;
}

interface Props {
  label: ReactNode;
  title?: string;
  currentBranch?: string | null;
  disabled?: boolean;
  anchor?: "top" | "bottom";
  align?: "left" | "right";
  /**
   * `select-only` — the default — renders branches as plain picker rows.
   * `manage` adds a right-click context menu on local-branch rows with
   * Checkout / Rename… / Delete actions (+ Delete (force) for unmerged
   * branches) and surfaces IntelliJ-style branch management.
   */
  mode?: "select-only" | "manage";
  /** Required when mode === "manage" for Merge/Rebase (operates on this stream's worktree). */
  streamId?: string | null;
  onPick(target: PickedRef): void | Promise<void>;
  buttonStyle?: CSSProperties;
}

/**
 * IntelliJ-style branch picker: button that opens a popover tree with
 * Recent / Local / Remote (grouped per-remote) / Tags sections and a filter
 * box. `onPick` runs when the user selects an entry; `currentBranch` is
 * highlighted as the active row.
 */
export function BranchPicker({
  label,
  title,
  currentBranch,
  disabled,
  anchor = "top",
  align = "right",
  mode = "select-only",
  streamId = null,
  onPick,
  buttonStyle,
}: Props) {
  const [open, setOpen] = useState(false);
  const [grouped, setGrouped] = useState<GroupedGitRefs | null>(null);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    recent: true,
    local: true,
    remote: true,
    tags: false,
  });
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [popoverCoords, setPopoverCoords] = useState<CSSProperties>({});
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; branch: BranchRef } | null>(null);
  const [renaming, setRenaming] = useState<{ from: string; value: string } | null>(null);
  const [deleting, setDeleting] = useState<{ branch: string; force: boolean; message: string } | null>(null);

  useEffect(() => {
    if (!open) return;
    function place() {
      const btn = btnRef.current;
      if (!btn) return;
      const rect = btn.getBoundingClientRect();
      const width = 340;
      const coords: CSSProperties = { position: "fixed", width };
      if (align === "right") {
        coords.left = Math.max(8, Math.min(window.innerWidth - width - 8, rect.right - width));
      } else {
        coords.left = Math.max(8, Math.min(window.innerWidth - width - 8, rect.left));
      }
      if (anchor === "top") {
        coords.bottom = window.innerHeight - rect.top + 4;
      } else {
        coords.top = rect.bottom + 4;
      }
      setPopoverCoords(coords);
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open, anchor, align]);

  useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => inputRef.current?.focus(), 0);
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    function onDocClick(e: MouseEvent) {
      const t = e.target as Node;
      if (popRef.current?.contains(t)) return;
      if (btnRef.current?.contains(t)) return;
      // Don't auto-close while a secondary UI (context menu, rename, delete
      // confirmation) is open on top of the popover — those render outside
      // popRef but are semantically part of this picker.
      if (contextMenu || renaming || deleting) return;
      setOpen(false);
    }
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDocClick);
    return () => {
      clearTimeout(t);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDocClick);
    };
  }, [open, contextMenu, renaming, deleting]);

  function openBranchMenu(event: ReactMouseEvent, branch: BranchRef) {
    event.preventDefault();
    event.stopPropagation();
    // Position the submenu to the LEFT of the picker popover, vertically
    // aligned with the clicked row. IntelliJ-style flyout. We measure the
    // popover's bounding rect so the menu sits flush against its left edge.
    const popRect = popRef.current?.getBoundingClientRect();
    const rowRect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const menuWidth = 240;
    const x = popRect ? Math.max(8, popRect.left - menuWidth - 4) : event.clientX;
    const y = rowRect.top;
    setContextMenu({ x, y, branch });
  }

  async function refresh() {
    try {
      const next = await listGitRefs();
      setGrouped(next);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRename() {
    if (!renaming) return;
    const to = renaming.value.trim();
    if (!to || to === renaming.from) {
      setRenaming(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = await renameGitBranch(renaming.from, to);
      if (!result.ok) setError((result.stderr || result.stdout || "rename failed").trim());
      else await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setRenaming(null);
    }
  }

  async function handleMerge(other: string) {
    if (!streamId) { setError("No stream selected"); return; }
    setBusy(true);
    setError(null);
    try {
      const result = await gitMergeInto(streamId, other);
      if (!result.ok) setError((result.stderr || result.stdout || "merge failed").trim());
      else { await refresh(); setOpen(false); }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleRebase(onto: string) {
    if (!streamId) { setError("No stream selected"); return; }
    setBusy(true);
    setError(null);
    try {
      const result = await gitRebaseOnto(streamId, onto);
      if (!result.ok) setError((result.stderr || result.stdout || "rebase failed").trim());
      else { await refresh(); setOpen(false); }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete() {
    if (!deleting) return;
    setBusy(true);
    setError(null);
    try {
      const result = await deleteGitBranch(deleting.branch, { force: deleting.force });
      if (!result.ok) {
        const msg = (result.stderr || result.stdout || "").trim();
        // Git signals "not fully merged" when refusing -d; offer a force path.
        if (!deleting.force && /not fully merged|is not fully merged/i.test(msg)) {
          setDeleting({
            branch: deleting.branch,
            force: true,
            message: `Branch "${deleting.branch}" is not fully merged. Delete anyway (will discard unmerged commits)?`,
          });
          return;
        }
        setError(msg || "delete failed");
      } else {
        await refresh();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      if (!deleting.force) setDeleting(null);
      else setDeleting(null);
    }
  }

  async function toggle() {
    if (disabled) return;
    if (open) { setOpen(false); return; }
    setOpen(true);
    setError(null);
    setFilter("");
    if (grouped) return;
    try {
      setLoading(true);
      const next = await listGitRefs();
      setGrouped(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  const q = filter.trim().toLowerCase();

  const filtered = useMemo(() => {
    if (!grouped) return { recent: [], local: [], remotes: [], tags: [] };
    const match = (s: string) => !q || s.toLowerCase().includes(q);
    return {
      recent: grouped.recent.filter(match),
      local: grouped.local.filter((b) => match(b.name)),
      remotes: grouped.remotes
        .map((g) => ({ remote: g.remote, branches: g.branches.filter((b) => match(b.name)) }))
        .filter((g) => g.branches.length > 0 || match(g.remote)),
      tags: grouped.tags.filter(match),
    };
  }, [grouped, q]);

  // When filtering, auto-expand all groups so matches are visible.
  const effectiveExpanded = useMemo<Record<string, boolean>>(() => {
    if (q) return { recent: true, local: true, remote: true, tags: true };
    return expanded;
  }, [expanded, q]);

  async function pickBranch(branch: BranchRef) {
    const localName = branch.kind === "local" ? branch.name : branch.name.split("/").slice(1).join("/");
    try {
      setBusy(true);
      setError(null);
      await onPick({ kind: "branch", name: localName, branch });
      setOpen(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function pickTag(tag: string) {
    try {
      setBusy(true);
      setError(null);
      await onPick({ kind: "tag", name: tag });
      setOpen(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function pickRecent(name: string) {
    const branch = grouped?.local.find((b) => b.name === name);
    if (branch) void pickBranch(branch);
  }

  function groupHeader(key: string, label: string, count: number, forceOpen?: boolean) {
    const isOpen = forceOpen ?? effectiveExpanded[key] ?? false;
    return (
      <button
        type="button"
        onClick={() => !forceOpen && setExpanded((p) => ({ ...p, [key]: !isOpen }))}
        style={groupHeaderStyle}
        disabled={!!forceOpen}
      >
        <span style={{ display: "inline-block", width: 10, color: "var(--muted)" }}>{isOpen ? "▾" : "▸"}</span>
        <span style={{ flex: 1, textAlign: "left", fontWeight: 600 }}>{label}</span>
        <span style={{ color: "var(--muted)", fontSize: 10 }}>{count}</span>
      </button>
    );
  }


  return (
    <span style={{ position: "relative", display: "inline-block" }}>
      <button
        ref={btnRef}
        type="button"
        onClick={() => void toggle()}
        disabled={disabled}
        title={title}
        style={{
          ...chipStyle,
          ...(buttonStyle ?? {}),
          cursor: disabled ? "default" : "pointer",
          opacity: disabled ? 0.6 : 1,
          background: open ? "var(--bg)" : (buttonStyle?.background ?? "transparent"),
        }}
      >
        {label}
      </button>
      {open ? (
        <div ref={popRef} style={{ ...popoverStyle, ...popoverCoords }}>
          <div style={popoverHeaderStyle}>
            <input
              ref={inputRef}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Search branches, tags…"
              style={inputStyle}
            />
          </div>
          {deleting ? (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "8px 10px",
                borderBottom: "1px solid var(--border)",
                background: "var(--bg-2)",
                fontSize: 12,
              }}
            >
              <span style={{ flex: 1, color: "var(--fg)" }}>{deleting.message}</span>
              <InlineConfirm
                triggerLabel={deleting.force ? "Force delete" : "Delete"}
                confirmLabel={deleting.force ? "Force delete" : "Delete"}
                onConfirm={() => { void handleDelete(); }}
              />
              <button
                type="button"
                onClick={() => setDeleting(null)}
                style={dialogButtonStyle}
              >
                Dismiss
              </button>
            </div>
          ) : null}
          <div style={listStyle}>
            {loading ? (
              <div style={emptyStyle}>Loading…</div>
            ) : !grouped ? (
              <div style={emptyStyle}>No data</div>
            ) : (
              <>
                {filtered.recent.length > 0 ? (
                  <div>
                    {groupHeader("recent", "Recent", filtered.recent.length)}
                    {effectiveExpanded.recent ? filtered.recent.map((name) => {
                      const branch = grouped?.local.find((b) => b.name === name);
                      const manageHere = mode === "manage" && !!branch;
                      return (
                        <RowButton
                          key={`recent:${name}`}
                          onClick={(e) => manageHere ? openBranchMenu(e, branch!) : pickRecent(name)}
                          disabled={busy}
                          current={name === currentBranch}
                          icon="⟲"
                          name={name}
                          meta="local"
                          showChevron={manageHere}
                        />
                      );
                    }) : null}
                  </div>
                ) : null}

                {filtered.local.length > 0 ? (
                  <div>
                    {groupHeader("local", "Local", filtered.local.length)}
                    {effectiveExpanded.local ? filtered.local.map((b) => {
                      const manageHere = mode === "manage";
                      return (
                        <RowButton
                          key={b.ref}
                          onClick={(e) => manageHere ? openBranchMenu(e, b) : void pickBranch(b)}
                          disabled={busy}
                          current={b.name === currentBranch}
                          icon="⎇"
                          name={b.name}
                          showChevron={manageHere}
                        />
                      );
                    }) : null}
                  </div>
                ) : null}

                {filtered.remotes.length > 0 ? (
                  <div>
                    {groupHeader("remote", "Remote", filtered.remotes.reduce((n, g) => n + g.branches.length, 0))}
                    {effectiveExpanded.remote ? filtered.remotes.map((g) => (
                      <RemoteGroup
                        key={g.remote}
                        remote={g.remote}
                        branches={g.branches}
                        busy={busy}
                        onPick={pickBranch}
                        onOpenMenu={mode === "manage" ? openBranchMenu : undefined}
                        forceOpen={!!q}
                      />
                    )) : null}
                  </div>
                ) : null}

                {filtered.tags.length > 0 ? (
                  <div>
                    {groupHeader("tags", "Tags", filtered.tags.length)}
                    {effectiveExpanded.tags ? filtered.tags.map((tag) => (
                      <RowButton
                        key={`tag:${tag}`}
                        onClick={() => void pickTag(tag)}
                        disabled={busy}
                        current={false}
                        icon="🏷"
                        name={tag}
                      />
                    )) : null}
                  </div>
                ) : null}

                {filtered.local.length === 0 && filtered.remotes.length === 0 && filtered.tags.length === 0 && filtered.recent.length === 0 ? (
                  <div style={emptyStyle}>No matches</div>
                ) : null}
              </>
            )}
          </div>
          {error ? <div style={errorStyle}>{error}</div> : null}
        </div>
      ) : null}
      {contextMenu ? (() => {
        const b = contextMenu.branch;
        const isLocal = b.kind === "local";
        const isCurrent = isLocal && b.name === currentBranch;
        const other = b.name; // short ref; git accepts e.g. "origin/main" directly
        return (
          <ContextMenu
            items={[
              {
                id: "branch.checkout",
                label: "Checkout",
                enabled: !busy && !isCurrent,
                run: () => { void pickBranch(b); },
              },
              {
                id: "branch.merge",
                label: currentBranch ? `Merge "${other}" into "${currentBranch}"` : `Merge "${other}" into current`,
                enabled: !busy && !isCurrent && !!streamId && !!currentBranch,
                run: () => { void handleMerge(other); },
              },
              {
                id: "branch.rebase",
                label: currentBranch ? `Rebase "${currentBranch}" onto "${other}"` : `Rebase current onto "${other}"`,
                enabled: !busy && !isCurrent && !!streamId && !!currentBranch,
                run: () => { void handleRebase(other); },
              },
              {
                id: "branch.rename",
                label: "Rename…",
                enabled: !busy && isLocal,
                run: () => setRenaming({ from: b.name, value: b.name }),
              },
              {
                id: "branch.delete",
                label: "Delete",
                enabled: !busy && isLocal && !isCurrent,
                run: () => setDeleting({
                  branch: b.name,
                  force: false,
                  message: `Delete branch "${b.name}"?`,
                }),
              },
            ]}
            position={{ x: contextMenu.x, y: contextMenu.y }}
            onClose={() => setContextMenu(null)}
            minWidth={220}
            zIndex={1250}
          />
        );
      })() : null}
      <Slideover
        open={!!renaming}
        onClose={() => setRenaming(null)}
        title={renaming ? `Rename branch "${renaming.from}"` : "Rename branch"}
        testId="branch-rename-slideover"
        footer={(
          <>
            <button type="button" onClick={() => setRenaming(null)} style={dialogButtonStyle}>Cancel</button>
            <button
              type="button"
              onClick={() => void handleRename()}
              style={dialogButtonStyle}
              disabled={busy || !renaming}
            >
              Rename
            </button>
          </>
        )}
      >
        {renaming ? (
          <form
            onSubmit={(e) => { e.preventDefault(); void handleRename(); }}
            style={{ display: "flex", flexDirection: "column", gap: 8 }}
          >
            <div style={{ fontSize: 12, color: "var(--muted)" }}>New name</div>
            <input
              autoFocus
              value={renaming.value}
              onChange={(e) => setRenaming({ ...renaming, value: e.target.value })}
              style={inputStyle}
            />
          </form>
        ) : null}
      </Slideover>
    </span>
  );
}

function RemoteGroup({ remote, branches, busy, onPick, onOpenMenu, forceOpen }: {
  remote: string;
  branches: BranchRef[];
  busy: boolean;
  onPick(branch: BranchRef): void | Promise<void>;
  onOpenMenu?(event: ReactMouseEvent, branch: BranchRef): void;
  forceOpen: boolean;
}) {
  const [open, setOpen] = useState(false);
  const isOpen = forceOpen || open;
  return (
    <div>
      <button type="button" onClick={() => setOpen((v) => !v)} style={{ ...groupHeaderStyle, paddingLeft: 18, fontWeight: 400 }}>
        <span style={{ display: "inline-block", width: 10, color: "var(--muted)" }}>{isOpen ? "▾" : "▸"}</span>
        <span style={{ flex: 1, textAlign: "left" }}>{remote}</span>
        <span style={{ color: "var(--muted)", fontSize: 10 }}>{branches.length}</span>
      </button>
      {isOpen ? branches.map((b) => {
        // Strip the "<remote>/" prefix for display — the group header already shows the remote.
        const display = b.name.startsWith(`${remote}/`) ? b.name.slice(remote.length + 1) : b.name;
        return (
          <RowButton
            key={b.ref}
            onClick={(e) => onOpenMenu ? onOpenMenu(e, b) : void onPick(b)}
            disabled={busy}
            current={false}
            icon="⎇"
            name={display}
            indent={30}
            showChevron={!!onOpenMenu}
          />
        );
      }) : null}
    </div>
  );
}

function RowButton({ onClick, disabled, current, icon, name, meta, indent = 18, showChevron = false }: {
  onClick(event: ReactMouseEvent): void;
  disabled: boolean;
  current: boolean;
  icon: string;
  name: string;
  meta?: string;
  indent?: number;
  showChevron?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={(e) => onClick(e)}
      disabled={disabled}
      style={{
        ...itemStyle,
        paddingLeft: indent,
        fontWeight: current ? 600 : 400,
        color: current ? "var(--fg)" : "var(--fg)",
        background: current ? "var(--bg-2)" : "transparent",
      }}
    >
      <span style={{ width: 14, color: "var(--muted)", fontSize: 11 }}>{icon}</span>
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{name}</span>
      {current ? <span style={{ fontSize: 10, color: "var(--accent)" }}>current</span> : null}
      {meta && !current ? <span style={{ fontSize: 10, color: "var(--muted)" }}>{meta}</span> : null}
      {showChevron ? <span style={{ fontSize: 10, color: "var(--muted)" }}>›</span> : null}
    </button>
  );
}

const chipStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  border: "1px solid transparent",
  borderRadius: 3,
  padding: "2px 8px",
  color: "var(--fg)",
  fontFamily: "inherit",
  fontSize: 11,
  height: 20,
};

const popoverStyle: CSSProperties = {
  maxHeight: 420,
  background: "var(--bg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 6,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.08), 0 12px 32px rgba(0,0,0,0.5)",
  display: "flex",
  flexDirection: "column",
  zIndex: 1200,
};

const popoverHeaderStyle: CSSProperties = {
  padding: 6,
  borderBottom: "1px solid var(--border)",
};

const inputStyle: CSSProperties = {
  width: "100%",
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "4px 6px",
  borderRadius: 3,
  fontFamily: "inherit",
  fontSize: 12,
  boxSizing: "border-box",
};

const listStyle: CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: 4,
  display: "flex",
  flexDirection: "column",
  gap: 1,
};

const groupHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  background: "transparent",
  border: "none",
  padding: "4px 8px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
  color: "var(--muted)",
  width: "100%",
  textAlign: "left",
};

const itemStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  background: "transparent",
  border: "none",
  padding: "3px 8px",
  borderRadius: 3,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
  textAlign: "left",
  color: "var(--fg)",
  width: "100%",
};

const emptyStyle: CSSProperties = {
  padding: "8px 10px",
  color: "var(--muted)",
  fontSize: 12,
};

const dialogButtonStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
};

const errorStyle: CSSProperties = {
  padding: "6px 10px",
  color: "#ff6b6b",
  fontSize: 11,
  borderTop: "1px solid var(--border)",
  whiteSpace: "pre-wrap",
};
