/// A lens's actions (P6.B1): each declares a command, run as the lens
/// acting for the person who pressed it — so it asks for confirmation when
/// the command does, and grants no power the person lacks. Copy and Add to
/// Agent Context are on every lens (kit affordances, not declared). See
/// `.context/extensions.md`.
import { formatContextMention } from "../agent-context-ref.js";
import { IpcCallError, needsConfirmation } from "../ipc-error.js";
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
  /** A toast; with `undo`, it offers Undo. */
  toast(message: string, undo?: () => void): void;
  /** Undo the run recorded as `auditId`, as the person. */
  undo(auditId: number): Promise<unknown>;
  recordError(label: string, message: string): void;
  /** The page the command opens after a run with `result` (its
   *  `ui.open_after`, bound by the server), or null. */
  openAfter(command: string, result: unknown): Promise<string | null>;
  /** Open `page` from the lens's page; `replace`: in its place, the lens's
   *  page leaving history. */
  openPage(page: string, replace: boolean): void;
}

/** What pressing an action came to: it ran, the command asks the person
 *  first (press again confirmed; `confirm` is what it asks about beyond
 *  itself, one line each), or it failed (recorded). */
export type LensActionOutcome = "done" | { confirm: string[] } | "failed";

/** A row as its column → value map (what a row action binds `{{row.*}}`
 *  to). */
export function rowRecord(columns: string[], row: SqlCell[]): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  columns.forEach((c, i) => {
    out[c] = row[i] ?? null;
  });
  return out;
}

/** Whether row action `action` is offered on `row`: always, unless its
 *  `when` names a column, which must then be truthy there (not null, 0,
 *  `""` or false). The server refuses it on any other row too. */
export function actionOffered(action: LensAction, row: Record<string, SqlCell>): boolean {
  if (!action.when) return true;
  const v = row[action.when];
  return v !== null && v !== undefined && v !== 0 && v !== "" && v !== false;
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
    const outcome = await deps.runLensAction(run.lens.id, action.id, run.params, row, streamId, confirmed);
    // A run that came back undoable offers its Undo.
    const auditId = outcome.inverse && outcome.audit_id != null ? outcome.audit_id : null;
    deps.toast(`${action.label}: done.`, auditId === null ? undefined : () => void deps.undo(auditId).catch((e: unknown) => deps.recordError(`Undo ${action.label}`, e instanceof Error ? e.message : String(e))));
    await openAfterAction(action, outcome, deps);
    return "done";
  } catch (e) {
    if (!confirmed && needsConfirmation(e)) return { confirm: e instanceof IpcCallError ? e.details : [] };
    deps.recordError(action.label, e instanceof Error ? e.message : String(e));
    return "failed";
  }
}

/** A command that declares where a person goes after it (`ui.open_after`)
 *  takes them there from a lens action too, as it does from a command
 *  offer — as the action's `then` says: `open` (Back returns), `replace`
 *  (the lens's page leaves history) or `stay` (nothing opens). A page the
 *  result can't name is the action's failure. */
export async function openAfterAction(
  action: Pick<LensAction, "label" | "command" | "then">,
  outcome: CommandOutcome,
  deps: Pick<LensActionDeps, "openAfter" | "openPage" | "recordError">,
): Promise<void> {
  if (action.then === "stay") return;
  try {
    const page = await deps.openAfter(action.command, outcome.result);
    if (page) deps.openPage(page, action.then === "replace");
  } catch (e) {
    deps.recordError(action.label, e instanceof Error ? e.message : String(e));
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
