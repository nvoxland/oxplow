import { useCallback, useEffect, useState } from "react";

import { listCommentsForTarget, onRemoteReconnect, subscribeCommentEvents } from "../../api.js";
import type { CommentThread } from "../../tauri-bridge/generated/bindings.js";

/// Fetch the comment threads anchored to one target URI (a wiki page, a
/// file, a work item) and keep them live: a commit that touches
/// `v_comment` triggers a refetch, so a reply landed by the agent (a
/// separate process) or another window shows up without a manual reload.
export function useCommentsForTarget(
  target: string,
): { threads: CommentThread[]; loading: boolean; reload: () => Promise<void> } {
  const [threads, setThreads] = useState<CommentThread[]>([]);
  const [loading, setLoading] = useState(true);

  const reload = useCallback(async () => {
    const list = await listCommentsForTarget(target);
    setThreads(list);
    setLoading(false);
  }, [target]);

  useEffect(() => {
    // No target → nothing to fetch (the field isn't comment-enabled).
    if (target === "") {
      setThreads([]);
      setLoading(false);
      return;
    }
    let active = true;
    setLoading(true);
    const fetch = async () => {
      const list = await listCommentsForTarget(target);
      if (active) {
        setThreads(list);
        setLoading(false);
      }
    };
    void fetch();
    const unsub = subscribeCommentEvents(() => void fetch());
    // Re-fetch after a remote-daemon WS reconnect so comment threads go
    // live again without a manual reload (events missed during the drop).
    const unsubReconnect = onRemoteReconnect(() => void fetch());
    return () => {
      active = false;
      unsub();
      unsubReconnect();
    };
  }, [target]);

  return { threads, loading, reload };
}
