import { useEffect, useState } from "react";
import { onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { coalesce, NO_READS, readsChanged } from "./viz/rerun.js";
import { NO_FEATURES, readTaskTrackerProfile, type TaskTrackerProfile } from "./tasks.js";

/**
 * The active task tracker's profile — which list, what it can do, its own
 * fields and ids — shared by every screen: read once, re-read when the
 * capability rows it read change (a switch). Every feature is off until
 * the first read lands.
 */

const LOADING: TaskTrackerProfile = { provider: null, features: NO_FEATURES, fields: [], stages: [], idPattern: null };

let current: TaskTrackerProfile = LOADING;
let reads = NO_READS;
let started = false;
/** The first read: what `loadTaskTrackerProfile` waits for. */
let first: Promise<void> | null = null;
const listeners = new Set<() => void>();

async function refresh(): Promise<void> {
  try {
    const out = await readTaskTrackerProfile();
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
  first = refresh();
  const rerun = coalesce(() => void refresh());
  subscribeOxplowEvents((event) => {
    if (readsChanged(event as Record<string, unknown>, reads)) rerun.schedule();
  });
  onRemoteReconnect(rerun.schedule);
}

/** The profile once it has been read (a component frame's markdown, which
 *  reads a list's ids by it). */
export async function loadTaskTrackerProfile(): Promise<TaskTrackerProfile> {
  start();
  await first;
  return current;
}

export function useTaskTrackerProfile(): TaskTrackerProfile {
  const [profile, setProfile] = useState<TaskTrackerProfile>(current);
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
