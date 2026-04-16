import { mkdirSync, readFileSync, readdirSync, rmSync, watch, type FSWatcher } from "node:fs";
import { join } from "node:path";
import type { Logger } from "../core/logger.js";
import type { PaneKind } from "../persistence/stream-store.js";

export interface HookEnvelope {
  event: string;
  streamId: string;
  batchId?: string;
  pane?: PaneKind;
  payload: unknown;
}

export class HookInbox {
  private watcher: FSWatcher | null = null;

  constructor(
    private readonly dir: string,
    private readonly onEnvelope: (envelope: HookEnvelope) => void,
    private readonly logger: Logger,
  ) {}

  start() {
    mkdirSync(this.dir, { recursive: true });
    this.processPending();
    this.watcher = watch(this.dir, () => {
      this.processPending();
    });
  }

  dispose() {
    this.watcher?.close();
    this.watcher = null;
  }

  private processPending() {
    for (const entry of readdirSync(this.dir)) {
      if (!entry.endsWith(".json")) continue;
      const path = join(this.dir, entry);
      try {
        const envelope = JSON.parse(readFileSync(path, "utf8")) as HookEnvelope;
        this.onEnvelope(envelope);
      } catch (error) {
        this.logger.warn("failed to process hook envelope", {
          file: entry,
          error: error instanceof Error ? error.message : String(error),
        });
      } finally {
        try { rmSync(path, { force: true }); } catch {}
      }
    }
  }
}
