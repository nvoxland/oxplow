import { useEffect, useState } from "react";
import { onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { knowledgeIsNone } from "./knowledge.js";
import { coalesce, NO_READS, readsChanged } from "./lens/lensRerun.js";
import { readCapabilityProviders } from "./workItems.js";

/**
 * Whether the project keeps no knowledge store (its active row is
 * `none`), shared by the wiki's surfaces: read once, re-read when the
 * capability rows it read change (a switch). False until the first read
 * lands.
 */

let none = false;
let reads = NO_READS;
let started = false;
const listeners = new Set<() => void>();

async function refresh(): Promise<void> {
  try {
    const out = await readCapabilityProviders("knowledge");
    none = knowledgeIsNone(out.providers);
    reads = out.reads;
    for (const fn of listeners) fn();
  } catch {
    // Keep what we had; the next change re-reads.
  }
}

function start(): void {
  if (started) return;
  started = true;
  void refresh();
  const rerun = coalesce(() => void refresh());
  subscribeOxplowEvents((event) => {
    if (readsChanged(event as Record<string, unknown>, reads)) rerun.schedule();
  });
  onRemoteReconnect(rerun.schedule);
}

export function useKnowledgeIsNone(): boolean {
  const [value, setValue] = useState(none);
  useEffect(() => {
    const update = () => setValue(none);
    listeners.add(update);
    start();
    update();
    return () => {
      listeners.delete(update);
    };
  }, []);
  return value;
}
