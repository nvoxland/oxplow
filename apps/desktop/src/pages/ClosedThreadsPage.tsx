import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useCallback, useEffect, useState } from "react";
import { Page } from "../tabs/Page.js";
import type { Stream, Thread } from "../tauri-bridge/index.js";
import { CANONICAL_STATES, STATE_LABEL, readWorkList, type WorkItem } from "../workItems.js";
import { readsChanged, readsOf } from "../lens/lensRerun.js";

const THREAD_READS = readsOf("v_thread");
import {
  listClosedThreads,
  reopenThread,
  subscribeOxplowEvents,
} from "../api.js";

export interface ClosedThreadsPageProps {
  stream: Stream | null;
  onAfterReopen?(threadId: string): void;
}

interface RowState {
  thread: Thread;
  items: WorkItem[];
  loading: boolean;
}

export function ClosedThreadsPage({ stream, onAfterReopen }: ClosedThreadsPageProps) {
  const [rows, setRows] = useState<RowState[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    if (!stream) {
      setRows([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const closed = await listClosedThreads(stream.id);
      const next: RowState[] = await Promise.all(
        closed.map(async (thread) => {
          try {
            const work = await readWorkList(thread.id);
            return { thread, items: work.all, loading: false };
          } catch {
            return { thread, items: [], loading: false };
          }
        }),
      );
      setRows(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [stream]);

  useEffect(() => {
    void refresh();
    const unsub = subscribeOxplowEvents((event) => {
      // A close or reopen commits to `v_thread`: re-list the closed threads.
      if (!readsChanged(event as Record<string, unknown>, THREAD_READS)) return;
      void refresh();
    });
    return unsub;
  }, [refresh, stream]);

  async function handleReopen(threadId: string) {
    if (!stream) return;
    try {
      await reopenThread(stream.id, threadId);
      onAfterReopen?.(threadId);
      void refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <Page title="Closed threads" kind="closed-threads" testId="closed-threads-page">
      {error ? <div style={{ color: "#ff6b6b", padding: "0 12px" }}>{error}</div> : null}
      {loading && rows.length === 0 ? (
        <div style={{ color: "var(--muted)", padding: 16 }}>Loading…</div>
      ) : rows.length === 0 ? (
        <div style={{ padding: "0 16px" }}>
          <EmptyState
            testId="closed-threads-empty"
            title="No closed threads"
            text="Threads you close from the rail's kebab menu show up here, with the work they finished."
          />
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 12, padding: 12 }}>
          {rows.map(({ thread, items }) => (
            <ClosedThreadRow
              key={thread.id}
              thread={thread}
              items={items}
              onReopen={() => void handleReopen(thread.id)}
            />
          ))}
        </div>
      )}
    </Page>
  );
}

function ClosedThreadRow({
  thread,
  items,
  onReopen,
}: {
  thread: Thread;
  items: WorkItem[];
  onReopen(): void;
}) {
  const grouped = groupByStatus(items);
  return (
    <div
      style={{
        border: "1px solid var(--border)",
        borderRadius: 8,
        background: "var(--bg-2)",
        padding: 12,
        display: "flex",
        flexDirection: "column",
        gap: 8,
      }}
      data-testid={`closed-thread-row-${thread.id}`}
    >
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
          <strong style={{ fontSize: "var(--text-base)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {thread.title}
          </strong>
          <span style={{ color: "var(--muted)", fontSize: 11 }}>
            closed {thread.closed_at ? new Date(thread.closed_at).toLocaleString() : "—"}
            {" · "}
            {items.length} item{items.length === 1 ? "" : "s"}
          </span>
        </div>
        <button
          type="button"
          onClick={onReopen}
          data-testid={`closed-thread-reopen-${thread.id}`}
          style={{
            border: "1px solid var(--border)",
            background: "var(--bg)",
            color: "inherit",
            borderRadius: 6,
            padding: "4px 10px",
            cursor: "pointer",
            fontFamily: "inherit",
            fontSize: "var(--text-xs)",
          }}
        >
          Reopen
        </button>
      </div>
      {items.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: "var(--text-xs)" }}>
          {CANONICAL_STATES.map((status) => {
            const bucket = grouped[status];
            if (!bucket || bucket.length === 0) return null;
            return (
              <div key={status} style={{ display: "flex", gap: 6, alignItems: "baseline" }}>
                <span style={{ color: "var(--muted)", textTransform: "uppercase", fontSize: 10, letterSpacing: 0.5, minWidth: 90 }}>
                  {STATE_LABEL[status]} ({bucket.length})
                </span>
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {bucket.map((i) => i.title).join(" · ")}
                </span>
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function groupByStatus(items: WorkItem[]): Record<string, WorkItem[]> {
  const out: Record<string, WorkItem[]> = {};
  for (const item of items) {
    (out[item.state] ??= []).push(item);
  }
  return out;
}
