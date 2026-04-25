/**
 * In-memory transient follow-up store.
 *
 * Follow-ups are tiny "I'll get back to that next" reminders the agent
 * stashes mid-turn when a deferred ask doesn't warrant a full work item
 * (no DB row, no review surface, just a bookmark for the next thing the
 * orchestrator wants to handle in the same conversation). Because they
 * carry zero durable value past the current session, this store is a
 * plain in-memory map — keyed by threadId, lost on runtime restart, no
 * SQLite involvement.
 *
 * The MCP tool surface (`add_followup` / `remove_followup` /
 * `list_followups`) is the agent-facing path. The UI surfaces the same
 * list inside `ThreadWorkState.followups` so the Work panel's To Do
 * section can render an italic muted reminder line per entry, with a
 * single ✕ dismiss button that calls remove via IPC.
 *
 * Subscribe to `subscribe()` to receive change notifications — the
 * runtime fans those out as `followup.changed` events on the bus so the
 * UI re-fetches the thread work state.
 */

export interface Followup {
  id: string;
  note: string;
  createdAt: string;
}

export interface FollowupChange {
  threadId: string;
  kind: "added" | "removed" | "cleared";
  id: string | null;
}

export class FollowupStore {
  private readonly byThread = new Map<string, Followup[]>();
  private readonly listeners = new Set<(change: FollowupChange) => void>();
  private nextSeq = 1;

  list(threadId: string): Followup[] {
    const arr = this.byThread.get(threadId);
    return arr ? arr.slice() : [];
  }

  add(threadId: string, note: string): Followup {
    const trimmed = note.trim();
    if (!trimmed) throw new Error("followup note must be non-empty");
    const id = `fu-${this.nextSeq++}-${Math.random().toString(36).slice(2, 8)}`;
    const entry: Followup = { id, note: trimmed, createdAt: new Date().toISOString() };
    const arr = this.byThread.get(threadId) ?? [];
    arr.push(entry);
    this.byThread.set(threadId, arr);
    this.emit({ threadId, kind: "added", id });
    return entry;
  }

  remove(threadId: string, id: string): boolean {
    const arr = this.byThread.get(threadId);
    if (!arr) return false;
    const idx = arr.findIndex((entry) => entry.id === id);
    if (idx < 0) return false;
    arr.splice(idx, 1);
    if (arr.length === 0) this.byThread.delete(threadId);
    this.emit({ threadId, kind: "removed", id });
    return true;
  }

  clear(threadId: string): void {
    if (!this.byThread.has(threadId)) return;
    this.byThread.delete(threadId);
    this.emit({ threadId, kind: "cleared", id: null });
  }

  subscribe(listener: (change: FollowupChange) => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private emit(change: FollowupChange): void {
    for (const listener of this.listeners) {
      try { listener(change); } catch { /* swallow — listeners shouldn't fault the store */ }
    }
  }
}
