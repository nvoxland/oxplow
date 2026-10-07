import { useEffect, useState } from "react";
import { onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { coalesce, NO_READS, readsChanged } from "./lens/lensRerun.js";
import { NO_FEATURES, readWorkListProfile, type WorkListProfile } from "./workItems.js";

/**
 * The active work list's profile — which list, what it can do, its own
 * fields and ids — shared by every screen: read once, re-read when the
 * capability rows it read change (a switch). Every feature is off until
 * the first read lands.
 */

const LOADING: WorkListProfile = { provider: null, features: NO_FEATURES, fields: [], idPattern: null };

let current: WorkListProfile = LOADING;
let reads = NO_READS;
let started = false;
const listeners = new Set<() => void>();

async function refresh(): Promise<void> {
  try {
    const out = await readWorkListProfile();
    current = out.profile;
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

export function useWorkListProfile(): WorkListProfile {
  const [profile, setProfile] = useState<WorkListProfile>(current);
  useEffect(() => {
    const update = () => setProfile(current);
    listeners.add(update);
    start();
    update();
    return () => {
      listeners.delete(update);
    };
  }, []);
  return profile;
}
