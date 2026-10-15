/// Where `window.show/navigate` takes the person: the stream and thread to
/// show and the page to open there. Pure over the window's thread lists;
/// `App.tsx` does it (`clientHost.ts`).
import { parseRef } from "../refs/ref.js";
import { refFromTabId } from "./pageRefs.js";
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

/** `ref` as a stream id: `stream:str1` or `str1`. */
function streamIdOf(ref: string): string {
  const parsed = parseRef(ref);
  return parsed && parsed.kind === "stream" ? parsed.id : ref;
}

/** What `input` (`{ ref, stream? }`) asks for, given the window's `states`
 *  and the stream it `shown`. Throws what it can't reach. */
export function planNavigation(
  input: unknown,
  states: Record<string, ThreadList | undefined>,
  shown: string | null,
): NavigationPlan {
  const { ref, stream } = (input ?? {}) as { ref?: unknown; stream?: unknown };
  if (typeof ref !== "string") throw new Error("`ref` is what to show (`file:src/a.rs`, `work_item:oxplow:tsk3`)");
  const known = (id: string) => {
    if (!states[id]) throw new Error(`no stream \`${id}\``);
    return id;
  };
  const selected = (id: string) => states[id]?.selectedThreadId ?? states[id]?.threads[0]?.id ?? null;

  const parsed = parseRef(ref);
  if (parsed?.kind === "stream") {
    const streamId = known(parsed.id);
    return { streamId, threadId: selected(streamId), selectThread: false, page: null };
  }
  if (parsed?.kind === "thread") {
    const streamId = Object.keys(states).find((id) => states[id]?.threads.some((t) => t.id === parsed.id));
    if (!streamId) throw new Error(`no thread \`${parsed.id}\``);
    return { streamId, threadId: parsed.id, selectThread: true, page: null };
  }
  const page = refFromTabId(ref);
  if (!page) throw new Error(`\`${ref}\` isn't something the window shows`);
  const named = typeof stream === "string" ? streamIdOf(stream) : shown;
  if (named === null) throw new Error("no stream is shown");
  const streamId = known(named);
  return { streamId, threadId: selected(streamId), selectThread: false, page };
}
