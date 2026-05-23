import { useEffect, useMemo, useState } from "react";

import { listCommentsForStream, subscribeCommentEvents } from "../api.js";
import { requestCommentReveal } from "../comment-reveal-bus.js";
import { resolvedWindowOptions, visibleThreads } from "../comments-filter.js";
import type { Stream } from "../api.js";
import type { CommentThread } from "../tauri-bridge/generated/bindings.js";
import { anchorIsApprox, CommentPopover } from "../components/Comments/CommentPopover.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { fileRef, taskRef, wikiPageRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";

/// Map a comment's target back to the page that owns it.
function targetRef(kind: string, id: string): TabRef | null {
  if (kind === "file") return fileRef(id);
  if (kind === "wiki") return wikiPageRef(id);
  if (kind === "task") {
    const n = Number(id);
    return Number.isFinite(n) ? taskRef(n) : null;
  }
  return null;
}

function targetLabel(kind: string, id: string): string {
  if (kind === "file") return id;
  if (kind === "wiki") return `wiki/${id}`;
  if (kind === "task") return `task #${id}`;
  return `${kind}:${id}`;
}

/// Global Comments inbox — every comment in the current stream, grouped
/// by the page it's anchored to. The holistic "look at them all" view:
/// clicking a comment row opens its full thread popover inline (read,
/// reply, set intent, resolve, delete) so the whole backlog can be
/// triaged from one place; clicking a group header jumps to the target
/// page where the anchored highlight lives. The agent reaches the same
/// data via the `list_comments` MCP tool.
export function CommentsInboxPage({
  stream,
  onOpenPage,
}: {
  stream: Stream | null;
  onOpenPage: (ref: TabRef) => void;
}) {
  usePageTitle("Comments Dashboard");
  const ctxNav = useOptionalPageNavigation();
  const [threads, setThreads] = useState<CommentThread[]>([]);
  const [active, setActive] = useState<{ id: number; rect: DOMRect } | null>(null);
  // null = unresolved only (default); a number N = also show threads
  // resolved within the last N days.
  const [resolvedWindowDays, setResolvedWindowDays] = useState<number | null>(null);
  const streamId = stream?.id ?? null;

  useEffect(() => {
    if (!streamId) {
      setThreads([]);
      return;
    }
    let active = true;
    const fetch = async () => {
      const list = await listCommentsForStream(streamId);
      if (active) setThreads(list);
    };
    void fetch();
    const unsub = subscribeCommentEvents(() => void fetch());
    return () => {
      active = false;
      unsub();
    };
  }, [streamId]);

  // Recompute the "now" baseline whenever the thread list changes so the
  // resolved-age buckets stay fresh without re-running every render.
  const nowMs = useMemo(() => Date.now(), [threads]);
  const resolvedOptions = useMemo(() => resolvedWindowOptions(threads, nowMs), [threads, nowMs]);
  const visible = useMemo(
    () => visibleThreads(threads, resolvedWindowDays, nowMs),
    [threads, resolvedWindowDays, nowMs],
  );

  const groups = useMemo(() => {
    const byTarget = new Map<string, { kind: string; id: string; threads: CommentThread[] }>();
    for (const t of visible) {
      const key = `${t.comment.target_kind}:${t.comment.target_id}`;
      let g = byTarget.get(key);
      if (!g) {
        g = { kind: t.comment.target_kind, id: t.comment.target_id, threads: [] };
        byTarget.set(key, g);
      }
      g.threads.push(t);
    }
    return [...byTarget.values()];
  }, [visible]);

  const navigate = (kind: string, id: string) => {
    const ref = targetRef(kind, id);
    if (!ref) return;
    if (ctxNav) ctxNav.navigate(ref, { newTab: false });
    else onOpenPage(ref);
  };

  // Open the comment's target page and ask that surface to scroll to and
  // open the anchored comment. The reveal request is stashed on the bus
  // because navigation is async — the target mounts and fetches its
  // threads after this returns (see comment-reveal-bus.ts).
  const goToLocation = (t: CommentThread) => {
    // Orphaned comments have no resolvable anchor to scroll to, so just
    // open the target page (where the user can re-select the text and
    // relink). Anchored ones also request a reveal to scroll/open inline.
    if (!t.comment.orphaned) requestCommentReveal(t.comment.id);
    navigate(t.comment.target_kind, t.comment.target_id);
  };

  return (
    <Page title="Comments Dashboard">
      <div style={{ padding: 24, display: "flex", flexDirection: "column", gap: 20 }} data-testid="page-comments">
        {resolvedOptions.length > 0 && (
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <label
              htmlFor="comments-resolved-filter"
              style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)", textTransform: "uppercase", letterSpacing: "0.04em" }}
            >
              Show
            </label>
            <select
              id="comments-resolved-filter"
              data-testid="comments-resolved-filter"
              value={resolvedWindowDays ?? ""}
              onChange={(e) =>
                setResolvedWindowDays(e.target.value === "" ? null : Number(e.target.value))
              }
              style={{
                background: "var(--surface-card)",
                color: "var(--text-primary)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
                padding: "4px 8px",
                fontSize: "var(--text-xs)",
              }}
            >
              <option value="">Unresolved only</option>
              {resolvedOptions.map((o) => (
                <option key={o.days} value={o.days}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
        )}
        {threads.length === 0 ? (
          <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
            No comments yet. Select text in a wiki page, file, or task and add one.
          </div>
        ) : groups.length === 0 ? (
          <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
            No unresolved comments.{resolvedOptions.length > 0 ? " Use the filter above to show resolved ones." : ""}
          </div>
        ) : (
          groups.map((g) => (
            <section key={`${g.kind}:${g.id}`} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <button
                type="button"
                data-testid={`comments-group-${g.kind}-${g.id}`}
                onClick={(e) => {
                  if (e.metaKey || e.ctrlKey) onOpenPage(targetRef(g.kind, g.id) ?? { id: "", kind: "agent", payload: null });
                  else navigate(g.kind, g.id);
                }}
                style={{
                  textAlign: "left",
                  background: "transparent",
                  border: "none",
                  color: "var(--text-secondary)",
                  fontSize: "var(--text-xs)",
                  textTransform: "uppercase",
                  letterSpacing: "0.04em",
                  cursor: "pointer",
                  padding: "4px 0",
                }}
              >
                {targetLabel(g.kind, g.id)} · {g.threads.length}
              </button>
              {g.threads.map((t) => {
                const lastMsg = t.messages[t.messages.length - 1];
                return (
                  // Card holds two distinct actions, so it's a div (not a
                  // button): the header carries "Go to location" and the
                  // body button opens the thread popover inline.
                  <div
                    key={t.comment.id}
                    data-testid={`comments-row-${t.comment.id}`}
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: 4,
                      padding: "8px 12px",
                      background: "var(--surface-card)",
                      border: "1px solid var(--border-subtle)",
                      borderRadius: 6,
                    }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span
                        style={{
                          fontSize: "var(--text-xs)",
                          padding: "1px 6px",
                          borderRadius: 4,
                          background: t.comment.intent === "followup" ? "var(--accent-soft-bg)" : "transparent",
                          color: t.comment.intent === "followup" ? "var(--accent)" : "var(--text-muted)",
                          border: "1px solid var(--border-subtle)",
                        }}
                      >
                        {t.comment.intent === "followup" ? "follow-up" : "note"}
                      </span>
                      {t.comment.status === "resolved" && (
                        <span style={{ fontSize: "var(--text-xs)", color: "var(--status-done)" }}>resolved</span>
                      )}
                      {t.comment.orphaned && (
                        <span style={{ fontSize: "var(--text-xs)", color: "var(--freshness-stale)" }}>orphaned</span>
                      )}
                      {!t.comment.orphaned && anchorIsApprox(t.comment.selectors_json) && (
                        <span
                          title="Re-attached approximately — the quoted text drifted, so this anchor may not be exact."
                          style={{ fontSize: "var(--text-xs)", color: "var(--freshness-stale)" }}
                        >
                          approx
                        </span>
                      )}
                      <span style={{ flex: 1 }} />
                      <button
                        type="button"
                        data-testid={`comments-goto-${t.comment.id}`}
                        title={
                          t.comment.orphaned
                            ? "Anchor lost — opens the target page so you can re-select the text and relink"
                            : "Open the page and scroll to this comment"
                        }
                        onClick={() => goToLocation(t)}
                        style={{
                          fontSize: "var(--text-xs)",
                          padding: "1px 8px",
                          borderRadius: 4,
                          background: "transparent",
                          color: "var(--accent)",
                          border: "1px solid var(--border-subtle)",
                          cursor: "pointer",
                        }}
                      >
                        {t.comment.orphaned ? "Go to page" : "Go to location"}
                      </button>
                    </div>
                    <button
                      type="button"
                      data-testid={`comments-open-${t.comment.id}`}
                      onClick={(e) =>
                        setActive({ id: t.comment.id, rect: e.currentTarget.getBoundingClientRect() })
                      }
                      style={{
                        textAlign: "left",
                        display: "flex",
                        flexDirection: "column",
                        gap: 4,
                        padding: 0,
                        background: "transparent",
                        border: "none",
                        color: "inherit",
                        cursor: "pointer",
                      }}
                    >
                      <div
                        style={{
                          fontSize: "var(--text-xs)",
                          color: "var(--text-secondary)",
                          fontStyle: "italic",
                          borderLeft: "2px solid var(--comment-highlight)",
                          paddingLeft: 8,
                        }}
                      >
                        “{t.comment.quote}”
                      </div>
                      {lastMsg && (
                        <div style={{ fontSize: "var(--text-sm)", color: "var(--text-primary)", whiteSpace: "pre-wrap" }}>
                          <span style={{ color: lastMsg.author === "agent" ? "var(--accent)" : "var(--text-secondary)", fontWeight: 600 }}>
                            {lastMsg.author}:{" "}
                          </span>
                          {lastMsg.body}
                        </div>
                      )}
                    </button>
                  </div>
                );
              })}
            </section>
          ))
        )}
      </div>
      {active &&
        (() => {
          const t = threads.find((x) => x.comment.id === active.id);
          return t ? (
            <CommentPopover thread={t} anchorRect={active.rect} onClose={() => setActive(null)} />
          ) : null;
        })()}
    </Page>
  );
}
