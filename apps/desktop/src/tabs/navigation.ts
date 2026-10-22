/// Where `window.show/navigate` takes the person: the stream and thread to
/// show and the page to open there. Pure over the window's thread lists;
/// `App.tsx` does it (`clientHost.ts`).
import { idOf } from "../uri/uri.js";
import { refTarget, type RefTarget } from "./refOpener.js";
import type { TabRef } from "./tabState.js";

interface ThreadList {
  selectedThreadId?: string | null;
  threads: Array<{ id: string }>;
}

export interface NavigationPlan {
  /** The stream to show (switched to when it isn't the one shown). */
  streamId: string;
  /** The thread the page opens in, and that is shown when the ref names it. */
  threadId: string | null;
  /** The ref named a thread: select it. */
  selectThread: boolean;
  /** The page to open and focus; none for a bare stream or thread. */
  page: TabRef | null;
}

/** `ref` as a stream id: a stream's URI or `str1`. */
function streamIdOf(ref: string): string {
  return idOf(ref, "oxplow/stream") ?? ref;
}

/** What `input` (`{ uri, stream? }`) asks for, given the window's `states`
 *  and the stream it `shown`: where the ref opens (`refOpener`). Throws
 *  what it can't reach. */
export async function planNavigation(
  input: unknown,
  states: Record<string, ThreadList | undefined>,
  shown: string | null,
  resolve: (ref: string) => Promise<RefTarget> = refTarget,
): Promise<NavigationPlan> {
  const { uri: ref, stream } = (input ?? {}) as { uri?: unknown; stream?: unknown };
  if (typeof ref !== "string") throw new Error("`uri` is the URI of what to show");
  const known = (id: string) => {
    if (!states[id]) throw new Error(`no stream \`${id}\``);
    return id;
  };
  const selected = (id: string) => states[id]?.selectedThreadId ?? states[id]?.threads[0]?.id ?? null;

  const target = await resolve(ref);
  if ("stream" in target) {
    const streamId = known(target.stream);
    return { streamId, threadId: selected(streamId), selectThread: false, page: null };
  }
  if ("thread" in target) {
    const threadId = target.thread;
    const streamId = Object.keys(states).find((id) => states[id]?.threads.some((t) => t.id === threadId));
    if (!streamId) throw new Error(`no thread \`${threadId}\``);
    return { streamId, threadId, selectThread: true, page: null };
  }
  if ("unopenable" in target) throw new Error(target.unopenable);
  const page = target.tab;
  const named = typeof stream === "string" ? streamIdOf(stream) : shown;
  if (named === null) throw new Error("no stream is shown");
  const streamId = known(named);
  return { streamId, threadId: selected(streamId), selectThread: false, page };
}
