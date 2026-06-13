import { useCallback, useEffect, useState } from "react";

import { listCommentsForTarget, onRemoteReconnect, subscribeCommentEvents } from "../../api.js";
import type { CommentThread } from "../../tauri-bridge/generated/bindings.js";

/// Fetch the comment threads anchored to one target (`wiki:<slug>`,
/// `file:<path>`, `task:<id>`) and keep them live: any `CommentsChanged`
/// event for this exact target triggers a refetch, so a reply landed by
/// the agent (a separate process) or another window shows up without a
/// manual reload.
export function useCommentsForTarget(
  targetKind: string,
  targetId: string,
): { threads: CommentThread[]; loading: boolean; reload: () => Promise<void> } {
  const [threads, setThreads] = useState<CommentThread[]>([]);
  const [loading, setLoading] = useState(true);

  const reload = useCallback(async () => {
    const list = await listCommentsForTarget(targetKind, targetId);
    setThreads(list);
    setLoading(false);
  }, [targetKind, targetId]);

  useEffect(() => {
    // No target → nothing to fetch (the field isn't comment-enabled).
    if (targetKind === "" || targetId === "") {
      setThreads([]);
      setLoading(false);
      return;
    }
    let active = true;
    setLoading(true);
    const fetch = async () => {
      const list = await listCommentsForTarget(targetKind, targetId);
      if (active) {
        setThreads(list);
        setLoading(false);
      }
    };
    void fetch();
    const unsub = subscribeCommentEvents(() => void fetch(), { targetKind, targetId });
    // Re-fetch after a remote-daemon WS reconnect so comment threads go
    // live again without a manual reload (events missed during the drop).
    const unsubReconnect = onRemoteReconnect(() => void fetch());
    return () => {
      active = false;
      unsub();
      unsubReconnect();
    };
  }, [targetKind, targetId]);

  return { threads, loading, reload };
}
