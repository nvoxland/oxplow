/// A lens's actions (P6.B1): each declares a command, run as the lens
/// acting for the person who pressed it — so it asks for confirmation when
/// the command does, and grants no power the person lacks. Copy and Add to
/// Agent Context are on every lens (kit affordances, not declared). See
/// `.context/extensions.md`.
import { formatContextMention } from "../agent-context-ref.js";
import { needsConfirmation } from "../ipc-error.js";
import type { CommandOutcome, LensAction, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

export interface LensActionDeps {
  runLensAction(
    id: string,
    action: string,
    params: Record<string, SqlCell>,
    row: Record<string, SqlCell> | null,
    streamId: string | null,
    confirmed: boolean,
  ): Promise<CommandOutcome>;
  toast(message: string): void;
  recordError(label: string, message: string): void;
}

/** What pressing an action came to: it ran, the command asks the person
 *  first (press again confirmed), or it failed (recorded). */
export type LensActionOutcome = "done" | "needs-confirmation" | "failed";

/** A row as its column → value map (what a row action binds `{{row.*}}`
 *  to). */
export function rowRecord(columns: string[], row: SqlCell[]): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  columns.forEach((c, i) => {
    out[c] = row[i] ?? null;
  });
  return out;
}

export async function performLensAction(
  action: LensAction,
  run: LensRun,
  row: Record<string, SqlCell> | null,
  streamId: string | null,
  confirmed: boolean,
  deps: LensActionDeps,
): Promise<LensActionOutcome> {
  try {
    await deps.runLensAction(run.lens.id, action.id, run.params, row, streamId, confirmed);
    deps.toast(`${action.label}: done.`);
    return "done";
  } catch (e) {
    if (!confirmed && needsConfirmation(e)) return "needs-confirmation";
    deps.recordError(action.label, e instanceof Error ? e.message : String(e));
    return "failed";
  }
}

/** Copy: the lens's text rendering, as an agent would read it. */
export async function copyLens(
  run: LensRun,
  streamId: string | null,
  deps: {
    lensText(id: string, params: Record<string, SqlCell>, streamId: string | null): Promise<string>;
    copyText(text: string): Promise<void>;
    recordError(label: string, message: string): void;
  },
): Promise<boolean> {
  try {
    await deps.copyText(await deps.lensText(run.lens.id, run.params, streamId));
    return true;
  } catch (e) {
    deps.recordError("Copy", e instanceof Error ? e.message : String(e));
    return false;
  }
}

/** Add to Agent Context: the lens and its params, into the agent's input
 *  (never sent). */
export function addLensToContext(run: LensRun, insertIntoAgent: (text: string) => void): void {
  insertIntoAgent(formatContextMention({ kind: "lens", lensId: run.lens.id, params: run.params }));
}
