import { useEffect, useState } from "react";
import { onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { coalesce, NO_READS, readsChanged } from "./viz/rerun.js";
import { readTaskTrackerProfiles, type TaskTrackerProfiles } from "./tasks.js";

/**
 * The task stores' profiles, by store id — each store's own stages, fields
 * and ids — shared by every screen: read once, re-read when the rows it
 * read change. A task's screen uses the profile of the task's own store
 * (`profileOfTask`). Empty until the first read lands.
 */

const LOADING: TaskTrackerProfiles = {};

let current: TaskTrackerProfiles = LOADING;
let reads = NO_READS;
let started = false;
/** The first read: what `loadTaskTrackerProfiles` waits for. */
let first: Promise<void> | null = null;
const listeners = new Set<() => void>();

async function refresh(): Promise<void> {
  try {
    const out = await readTaskTrackerProfiles();
    current = out.profiles;
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
export async function loadTaskTrackerProfiles(): Promise<TaskTrackerProfiles> {
  start();
  await first;
  return current;
}

export function useTaskTrackerProfiles(): TaskTrackerProfiles {
  const [profile, setProfile] = useState<TaskTrackerProfiles>(current);
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
