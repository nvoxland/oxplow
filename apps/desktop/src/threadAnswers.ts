/**
 * A thread's answers (P6.C2, `.context/extensions.md` → "Thread answers"):
 * the lenses an agent showed with `oxplow_show_lens`, read from `v_thread_answer`.
 * A read returns what it read (`reads`) so the strip re-runs with
 * `useRerunOnChange`; each answer re-runs itself through `runAnswer`.
 */
import { keptLens, querySql, runCommand, type KeptLens, type SqlCell } from "./api.js";
import { idOf } from "./uri/uri.js";
import { threadRowId } from "./modelIds.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";

export interface AnswerRow {
  /** `answer:<id>`. */
  ref: string;
  title: string;
  /** The existing lens it shows, if it shows one. */
  lens: string | null;
  /** The lens it was kept as (`oxplow.lens.keep`), once it was. */
  keptLens: string | null;
}

const ANSWERS_SQL = `SELECT ref, title, lens, kept_lens FROM v_thread_answer
                      WHERE thread_id = ?1 ORDER BY id DESC`;

export function answersFromRows(rows: SqlCell[][]): AnswerRow[] {
  return rows.map(([ref, title, lens, kept]) => ({
    ref: String(ref),
    title: String(title ?? ""),
    lens: lens === null ? null : String(lens),
    keptLens: kept === null ? null : String(kept),
  }));
}

/** A thread's answers, newest first. */
export async function readAnswers(threadId: string): Promise<{ answers: AnswerRow[]; reads: Reads }> {
  const res = await querySql(ANSWERS_SQL, [threadRowId(threadId)]);
  return { answers: answersFromRows(res.rows), reads: res.reads };
}

/** Keep This: the answer becomes a private lens (`oxplow.lens.keep`) in its
 *  thread's worktree; an empty slug lets the command take one from the
 *  title. Returns the lens id and whether the app shows it now. */
export async function keepAnswer(answer: string, slug: string): Promise<KeptLens> {
  const id = idOf(answer, "oxplow/answer");
  if (id === null) throw new Error(`not an answer: ${answer}`);
  const input: Record<string, string | number> = { answer: Number(id) };
  if (slug.trim() !== "") input.slug = slug.trim();
  const outcome = await runCommand("oxplow.lens.keep", input);
  return keptLens(outcome.result);
}
