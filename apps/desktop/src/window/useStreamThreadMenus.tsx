import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";

import { archiveStream, type Stream, type Thread, type ThreadMark, type ThreadState } from "../api.js";
import type { AgentStatusDotState } from "../components/AgentStatusDot.js";
import { refCommandMenuItems, useRefOffers } from "../components/refCommands.js";
import { useContextMenu, type MenuItems } from "../components/useRowContextMenu.js";
import type { MenuItem } from "../menu.js";
import { subscribeNavigatorMenuRequests, type NavigatorMenuRequest } from "../navigator-bus.js";
import { subscribeNewThreadRequests } from "../new-thread-bus.js";
import { openInForPath } from "../openIn.js";
import { threadUri } from "../uri/uri.js";

/** The colours a person marks a thread with, in menu order. */
export const THREAD_MARKS: ThreadMark[] = ["amber", "red", "green", "blue", "violet"];

export const MARK_LABELS: Record<ThreadMark, string> = {
  amber: "Amber",
  red: "Red",
  green: "Green",
  blue: "Blue",
  violet: "Violet",
};

/** A mark's colour token (`--thread-mark-<colour>`, theming.md). */
export function markColour(mark: ThreadMark): string {
  return `var(--thread-mark-${mark})`;
}

export type RenameTarget = { kind: "stream" | "thread"; id: string };

export interface StreamThreadMenusInput {
  streams: Stream[];
  threadStates: Record<string, ThreadState>;
  streamStatuses: Record<string, AgentStatusDotState>;
  currentStreamId: string | null;
  onRenameStream?(streamId: string, title: string): void | Promise<void>;
  onRenameThread?(threadId: string, title: string): void | Promise<void>;
  onMarkThread?(threadId: string, mark: ThreadMark | null): void | Promise<void>;
  onCloseThread?(threadId: string): void | Promise<void>;
  onOpenStreamSettings?(streamId: string): void;
  onOpenThreadSettings?(threadId: string): void;
  /** Show the place where this shell draws the flows (the Navigator's
   *  panel): a flow started from a menu or a request shows there. */
  reveal(): void;
}

export interface StreamRemoval {
  /** The stream being removed, asked about first; null when none. */
  stream: Stream | null;
  /** Also delete its worktree. */
  worktree: boolean;
  setWorktree(on: boolean): void;
  busy: boolean;
  error: string | null;
  confirm(): Promise<void>;
  cancel(): void;
}

/**
 * The menus of a window's streams and threads, and the flows they start
 * (rename, add a thread, remove a stream), for any shell: the desktop's
 * Navigator draws them in its panel, a shell without one draws them where
 * it shows streams. It also answers the window's requests: New Thread
 * (`new-thread-bus`) and a menu asked for from outside (`navigator-bus`,
 * the title bar's names), drawn by `requestedMenu`. One menu per stream
 * and per thread wherever it opens.
 */
export function useStreamThreadMenus(input: StreamThreadMenusInput): {
  streamMenu(s: Stream): MenuItem[];
  threadMenu(t: Thread): Promise<MenuItem[]>;
  renaming: RenameTarget | null;
  setRenaming(t: RenameTarget | null): void;
  pendingNewThreadFor: string | null;
  setPendingNewThreadFor(streamId: string | null): void;
  removal: StreamRemoval;
  /** The menu a request from outside opened: render it once. */
  requestedMenu: ReactNode;
} {
  const { streams, threadStates, streamStatuses, currentStreamId } = input;
  const [pendingNewThreadFor, setPendingNewThreadFor] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<RenameTarget | null>(null);
  const [removeStream, setRemoveStream] = useState<Stream | null>(null);
  const [removeWorktree, setRemoveWorktree] = useState(false);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  // The latest input, for the subscriptions (made once).
  const latest = useRef(input);
  latest.current = input;

  // The command palette's New Thread… (and any other surface): show the
  // flow for the requested stream.
  useEffect(
    () =>
      subscribeNewThreadRequests((streamId) => {
        latest.current.reveal();
        setPendingNewThreadFor(streamId);
      }),
    [],
  );

  const refOffersFor = useRefOffers();
  const streamMenu = (s: Stream): MenuItem[] => {
    const items: MenuItem[] = [
      {
        id: "stream.add-thread",
        label: "Add thread",
        enabled: true,
        run: () => {
          input.reveal();
          setPendingNewThreadFor(s.id);
        },
      },
      {
        id: "stream.rename",
        label: "Rename…",
        enabled: !!input.onRenameStream,
        run: () => {
          input.reveal();
          setRenaming({ kind: "stream", id: s.id });
        },
      },
      {
        id: "stream.settings",
        label: "Settings…",
        enabled: !!input.onOpenStreamSettings,
        run: () => input.onOpenStreamSettings?.(s.id),
      },
    ];
    // Its worktree outside oxplow (`openIn.ts`); Terminal only for this
    // window's stream, where a new terminal opens.
    const elsewhere = s.worktree_path ? openInForPath("dir", s.worktree_path, [], { terminal: s.id === currentStreamId }) : null;
    if (elsewhere) items.push(elsewhere);
    if (s.kind !== "primary") {
      items.push({
        id: "stream.remove",
        label: "Remove…",
        // Not while an agent works in one of its threads: the IPC refuses
        // too, but a disabled item saves a useless question.
        enabled: streamStatuses[s.id] !== "working",
        run: () => {
          setRemoveStream(s);
          setRemoveWorktree(false);
          setRemoveError(null);
        },
      });
    }
    return items;
  };
  const threadMenu = async (thread: Thread): Promise<MenuItem[]> => {
    const markItem = (mark: ThreadMark | null): MenuItem => ({
      id: `thread.mark.${mark ?? "none"}`,
      label: mark ? MARK_LABELS[mark] : "None",
      enabled: true,
      checked: (thread.mark ?? null) === mark,
      swatch: mark ? markColour(mark) : undefined,
      run: () => input.onMarkThread?.(thread.id, mark),
    });
    return [
      {
        id: "thread.mark",
        label: "Mark with…",
        enabled: !!input.onMarkThread,
        submenuHeader: "A colour on the thread's tab, for grouping threads your way. It changes nothing else.",
        submenu: [...THREAD_MARKS.map(markItem), markItem(null)],
      },
      {
        id: "thread.rename",
        label: "Rename…",
        enabled: !!input.onRenameThread,
        run: () => {
          input.reveal();
          setRenaming({ kind: "thread", id: thread.id });
        },
      },
      {
        id: "thread.settings",
        label: "Settings…",
        enabled: !!input.onOpenThreadSettings,
        run: () => input.onOpenThreadSettings?.(thread.id),
      },
      {
        id: "thread.close",
        label: "Close thread",
        enabled: !!input.onCloseThread,
        run: () => input.onCloseThread?.(thread.id),
      },
      // Then the commands about the thread (usability.md "Menus a page or
      // row gets from extensions").
      ...refCommandMenuItems(await refOffersFor(threadUri(thread.id))),
    ];
  };

  // A stream's or thread's menu asked for from outside (the title bar's
  // names): the same menu, headed the same way, at the asker's point.
  const requested = useContextMenu();
  const menuFor = (r: NavigatorMenuRequest): { items: MenuItems; header: string } | null => {
    if (r.kind === "stream") {
      const s = streams.find((x) => x.id === r.id);
      return s ? { items: streamMenu(s), header: s.title } : null;
    }
    for (const ts of Object.values(threadStates)) {
      const t = ts.threads.find((x) => x.id === r.id);
      if (t) return { items: threadMenu(t), header: t.title };
    }
    return null;
  };
  const menuForRef = useRef(menuFor);
  menuForRef.current = menuFor;
  const openAt = requested.openAt;
  useEffect(
    () =>
      subscribeNavigatorMenuRequests((r) => {
        const menu = menuForRef.current(r);
        if (menu) openAt({ x: r.x, y: r.y }, menu.items, menu.header);
      }),
    [openAt],
  );

  const removal: StreamRemoval = {
    stream: removeStream,
    worktree: removeWorktree,
    setWorktree: setRemoveWorktree,
    busy: removeBusy,
    error: removeError,
    confirm: async () => {
      if (!removeStream) return;
      try {
        setRemoveBusy(true);
        setRemoveError(null);
        await archiveStream(removeStream.id, removeWorktree);
        setRemoveStream(null);
      } catch (e) {
        setRemoveError(String(e));
      } finally {
        setRemoveBusy(false);
      }
    },
    cancel: () => {
      if (!removeBusy) setRemoveStream(null);
    },
  };

  return { streamMenu, threadMenu, renaming, setRenaming, pendingNewThreadFor, setPendingNewThreadFor, removal, requestedMenu: requested.menu };
}
