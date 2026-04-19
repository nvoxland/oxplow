import type { Logger } from "../core/logger.js";

/**
 * Shared subscribe/emit machinery for the persistence stores. Pulls the
 * six near-identical pub/sub blocks (batch, commit-point, file-change,
 * turn, wait-point, work-item) into one place so:
 *
 *   - bug fixes (snapshot iteration, throwing-listener handling) land in
 *     one spot rather than six;
 *   - the iteration is **snapshotted** so a listener that unsubscribes
 *     itself (or another listener) during emission doesn't skip
 *     subsequent listeners;
 *   - thrown listeners are logged with a consistent label per store and
 *     iteration continues for the rest.
 *
 * Stores compose this rather than inheriting from it so they keep their
 * existing constructor shape (the SQLite handle is pulled in their
 * constructor, not ours).
 */
export class StoreEmitter<Change> {
  private readonly listeners = new Set<(change: Change) => void>();

  constructor(
    private readonly label: string,
    private readonly logger?: Logger,
  ) {}

  subscribe(listener: (change: Change) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  emit(change: Change): void {
    // Snapshot before iterating: a listener that calls its own unsubscribe
    // (common one-shot pattern) would otherwise mutate the live Set during
    // iteration. Set iteration order is technically defined but skipping
    // listeners is a real foot-gun.
    for (const listener of [...this.listeners]) {
      try {
        listener(change);
      } catch (error) {
        this.logger?.warn(`${this.label} listener threw`, {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }
}
