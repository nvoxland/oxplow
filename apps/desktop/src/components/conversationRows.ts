/**
 * The rows of the conversation picker (`ConversationPicker.tsx`): an
 * agent session's harness's saved conversations, for the person to resume
 * one in the session or fork one into it (`.context/terminal.md`
 * "Choosing another conversation").
 */
import type { ListedConversation } from "../tauri-bridge/generated/bindings.js";

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

function size(bytes: number | null): string | null {
  if (bytes === null) return null;
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function when(at: string | null): string | null {
  if (!at) return null;
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function conversationRows(listed: readonly ListedConversation[], mode: PickerMode): ConversationRow[] {
  return listed.map(({ conversation: c, current, open_in }) => ({
    id: c.id,
    label: c.title?.trim() || c.first_prompt?.trim() || c.id,
    when: when(c.updated_at),
    size: size(c.size_bytes),
    note: current ? "this session's" : open_in ? "open in another session" : null,
    takeable: mode === "fork" || (!current && !open_in),
  }));
}
