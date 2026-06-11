/// Per-document didChange bookkeeping for the LSP full-text sync:
/// a monotonically increasing version per open path (LSP versions must
/// only go up — Monaco's getVersionId resets when models are swapped,
/// which is the bug this replaces) and a debounce so keystrokes don't
/// each produce a notification. Pure logic — the caller supplies the
/// send function — so it tests without Monaco or a bridge.

interface DocState {
  version: number;
  lastSentText: string;
  pendingText: string | null;
  timer: ReturnType<typeof setTimeout> | null;
}

export class DocumentSyncTracker {
  private docs = new Map<string, DocState>();

  constructor(
    private readonly send: (path: string, text: string, version: number) => void,
    private readonly debounceMs = 200,
  ) {}

  /// Start tracking an opened document. Returns the version the caller
  /// should put in `didOpen` (always 1 for a fresh open).
  open(path: string, text: string): number {
    this.clearTimer(this.docs.get(path));
    this.docs.set(path, { version: 1, lastSentText: text, pendingText: null, timer: null });
    return 1;
  }

  /// Buffer content changed; schedules a debounced didChange.
  changed(path: string, text: string): void {
    const doc = this.docs.get(path);
    if (!doc) return;
    if (text === doc.lastSentText && doc.pendingText === null) return;
    doc.pendingText = text;
    if (doc.timer) clearTimeout(doc.timer);
    doc.timer = setTimeout(() => this.flush(path), this.debounceMs);
  }

  /// Send any pending didChange for `path` immediately. Call before
  /// didSave and before any positional request (hover/completion/…) so
  /// the server never answers against stale text.
  flush(path: string): void {
    const doc = this.docs.get(path);
    if (!doc) return;
    this.clearTimer(doc);
    if (doc.pendingText === null || doc.pendingText === doc.lastSentText) {
      doc.pendingText = null;
      return;
    }
    doc.version += 1;
    doc.lastSentText = doc.pendingText;
    doc.pendingText = null;
    this.send(path, doc.lastSentText, doc.version);
  }

  close(path: string): void {
    const doc = this.docs.get(path);
    this.clearTimer(doc);
    this.docs.delete(path);
  }

  reset(): void {
    for (const doc of this.docs.values()) this.clearTimer(doc);
    this.docs.clear();
  }

  isTracking(path: string): boolean {
    return this.docs.has(path);
  }

  private clearTimer(doc: DocState | undefined): void {
    if (doc?.timer) {
      clearTimeout(doc.timer);
      doc.timer = null;
    }
  }
}
