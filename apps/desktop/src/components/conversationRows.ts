/**
 * The rows of the conversation picker (`ConversationPicker.tsx`): an
 * agent session's harness's saved conversations, for the person to resume
 * one in the session or fork one into it (`.context/terminal.md`
 * "Choosing another conversation").
 */
import type { ListedConversation } from "../tauri-bridge/generated/bindings.js";
import { formatSize, formatWhen } from "./conversationFormat.js";

export type PickerMode = "resume" | "fork";

export interface ConversationRow {
  id: string;
  /** Its title, else the first thing the person said, else its id. */
  label: string;
  /** When it last changed (`YYYY-MM-DD HH:MM`, local), when known. */
  when: string | null;
  size: string | null;
  /** Why it stands out: the session's own, or open in another. */
  note: string | null;
  /** It can be taken in this mode: resuming takes a free one other than
   *  the session's own; forking copies any. */
  takeable: boolean;
}

export function conversationRows(listed: readonly ListedConversation[], mode: PickerMode): ConversationRow[] {
  return listed.map(({ conversation: c, current, open_in }) => ({
    id: c.id,
    label: c.title?.trim() || c.first_prompt?.trim() || c.id,
    when: formatWhen(c.updated_at),
    size: formatSize(c.size_bytes),
    note: current ? "this session's" : open_in ? "open in another session" : null,
    takeable: mode === "fork" || (!current && !open_in),
  }));
}
