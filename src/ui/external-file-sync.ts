import type { OpenFileState } from "../session/file-session.js";

export type ExternalFileSyncAction = "noop" | "update-saved" | "replace-draft" | "prompt";

export function externalFileSyncAction(openFile: OpenFileState | null, diskContent: string): ExternalFileSyncAction {
  if (!openFile || openFile.isLoading) return "noop";
  if (diskContent === openFile.savedContent) return "noop";
  if (diskContent === openFile.draftContent) return "update-saved";
  if (openFile.draftContent !== openFile.savedContent) return "prompt";
  return "replace-draft";
}
