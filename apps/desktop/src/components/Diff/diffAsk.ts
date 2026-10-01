/// Ask About This on a diff (P6.D1): what the right side's selection names
/// — its file at the right side's revision (null = the working tree). A
/// literal right side (a compare with the clipboard) names nothing, so the
/// pane doesn't offer the action there.
import { revisionSlot, type Revision } from "../../revision.js";

export function diffAskTarget(spec: {
  path: string;
  rightVersion: Revision;
  rightContent?: string;
}): { path: string; rev: string | null } | null {
  if (spec.rightContent !== undefined) return null;
  return { path: spec.path, rev: revisionSlot(spec.rightVersion) };
}
