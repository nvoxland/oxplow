/// A viz's actions (P6.B1): each declares a command, run as the page
/// acting for the person who pressed it — so it asks for confirmation when
/// the command does, and grants no power the person lacks. Copy is the
/// page's (`page_text`), Add to Context its menu's. See
/// `.context/extensions-pages.md`.
import { fieldsOf, secretFields, shownFields, type Field } from "../components/SchemaForm/schemaFormModel.js";
import { IpcCallError, needsConfirmation } from "../ipc-error.js";
import type { CommandOutcome, Drop, FormStart, PageAction, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { ranMessage, resultUri } from "../commandToast.js";

export interface VizActionDeps {
  /** Press it on the page run the viz is in. */
  press(
    id: string,
    action: string,
    params: Record<string, SqlCell>,
    row: Record<string, SqlCell> | null,
    streamId: string | null,
    confirmed: boolean,
    /** Where the dragged row landed, for a drag action. */
    drop?: Drop | null,
  ): Promise<CommandOutcome>;
  /** A toast; with `undo`, it offers Undo. */
  toast(message: string, undo?: () => void, open?: () => void): void;
  /** Undo the run recorded as `auditId`, as the person. */
  undo(auditId: number): Promise<unknown>;
  recordError(label: string, message: string): void;
  /** The page the command opens after a run with `result` (its
   *  `ui.open_after`, bound by the server), or null. */
  openAfter(command: string, result: unknown): Promise<string | null>;
  /** Open `page` from the viz's page; `replace`: in its place, the viz's
   *  page leaving history. */
  openPage(page: string, replace: boolean): void;
  /** Close the viz's page (`then: close`). */
  closePage?(): void;
}

/** What pressing an action came to: it ran, the command asks the person
 *  first (press again confirmed; `confirm` is what it asks about beyond
 *  itself, one line each), or it failed (recorded). */
export type VizActionOutcome = "done" | { confirm: string[] } | "failed";

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
export function actionOffered(action: PageAction, row: Record<string, SqlCell>): boolean {
  if (!action.when) return true;
  return truthyCell(row[action.when]);
}

/** A cell that counts as true for a `when`: not null, 0, `""` or false. */
export function truthyCell(v: SqlCell | undefined): boolean {
  return v !== null && v !== undefined && v !== 0 && v !== "" && v !== false;
}

/** Whether whole-page action `action` is offered as a button over `result`:
 *  not a row, group or edit action (those live on rows, headings and the
 *  editor); an ordinary button always; one that names the viz's one row
 *  (`oneRow`: `when` or `{{row.*}}`) only while the result has exactly one
 *  row and, if it has a `when`, that column is truthy on it. The server
 *  reads and checks the same row when it runs. */
export function offeredWholePage(action: PageAction, result: Pick<VizRun["result"], "columns" | "rows">): boolean {
  if (action.row || action.group || action.edit) return false;
  if (!action.oneRow) return true;
  if (result.rows.length !== 1) return false;
  return actionOffered(action, rowRecord(result.columns, result.rows[0]!));
}

/** What an action is pressed on: the viz (only its id) and the params it
 *  ran with. A page file's whole-page actions are pressed on its page. */
export interface ActionTarget {
  config: Pick<VizRun["config"], "id">;
  params: Record<string, SqlCell>;
}

/** What an action does, in a word for a title: the command it runs, or
 *  the column whose URI it opens. */
export function actionDoes(action: Pick<PageAction, "command" | "open">): string {
  return action.open ? `opens ${action.open}` : (action.command ?? "");
}

export async function performVizAction(
  action: PageAction,
  run: ActionTarget,
  row: Record<string, SqlCell> | null,
  streamId: string | null,
  confirmed: boolean,
  deps: VizActionDeps,
  drop: Drop | null = null,
): Promise<VizActionOutcome> {
  try {
    const outcome = await deps.press(run.config.id, action.id, run.params, row, streamId, confirmed, drop);
    // An `open` action ran nothing: it says the URI to open, as a link does.
    if (action.open) {
      const uri = (outcome.result as { opened?: unknown } | null)?.opened;
      if (typeof uri !== "string") throw new Error(`${action.label}: the row has no URI to open`);
      deps.openPage(uri, false);
      return "done";
    }
    // A run that came back undoable offers its Undo.
    const auditId = outcome.inverse && outcome.audit_id != null ? outcome.audit_id : null;
    const uri = resultUri(outcome.result);
    deps.toast(
      ranMessage(action.label, outcome.result),
      auditId === null ? undefined : () => void deps.undo(auditId).catch((e: unknown) => deps.recordError(`Undo ${action.label}`, e instanceof Error ? e.message : String(e))),
      uri === null ? undefined : () => deps.openPage(uri, false),
    );
    await openAfterAction(action, outcome, deps);
    return "done";
  } catch (e) {
    if (!confirmed && needsConfirmation(e)) return { confirm: e instanceof IpcCallError ? e.details : [] };
    deps.recordError(action.label, e instanceof Error ? e.message : String(e));
    return "failed";
  }
}

/** A command that declares where a person goes after it (`ui.open_after`)
 *  takes them there from a viz action too, as it does from a command
 *  offer — as the action's `then` says: `open` (Back returns), `replace`
 *  (the viz's page leaves history), `stay` (nothing opens) or `close`
 *  (the viz's page closes). A page the
 *  result can't name is the action's failure. */
export async function openAfterAction(
  action: Pick<PageAction, "label" | "command" | "then">,
  outcome: CommandOutcome,
  deps: Pick<VizActionDeps, "openAfter" | "openPage" | "closePage" | "recordError">,
): Promise<void> {
  if (action.then === "stay") return;
  if (action.then === "close") {
    deps.closePage?.();
    return;
  }
  try {
    const page = await deps.openAfter(action.command ?? "", outcome.result);
    if (page) deps.openPage(page, action.then === "replace");
  } catch (e) {
    deps.recordError(action.label, e instanceof Error ? e.message : String(e));
  }
}

/** The one text field a form action's form asks for, when that is all it
 *  asks (its fields, narrowed to `start.fields` when set, less the fixed
 *  ones): such a form is a prompt strip at the top of the viz, not a
 *  slideover. Anything else — two fields, a choice, a field picked from a
 *  list (`choices`), markdown — is null.
 *  `secret`: the field is a secret (`secretFields`), masked and never
 *  required. */
export function promptField(start: FormStart): (Field & { secret: boolean }) | null {
  const schema = start.command.input_schema as Record<string, unknown>;
  const all = fieldsOf(schema);
  const specs = start.fields ? shownFields(start.fields) : null;
  const listed = specs ? specs.flatMap((spec) => all.find((f) => f.path === spec.name) ?? []) : all;
  const fixed = new Set(start.fixed);
  const editable = listed.filter((f) => !fixed.has(f.path));
  const only = editable.length === 1 ? editable[0]! : null;
  // A field picked from a list is a select: the slideover's, never a strip.
  if (only && start.choices?.[only.path]) return null;
  return only?.kind === "text" ? { ...only, secret: secretFields(schema, specs).has(only.path) } : null;
}
