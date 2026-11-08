/// Commands a person runs from outside a page (P6.D1: a launcher entry).
/// A command that asks for confirmation waits here until the person
/// answers in `PersonCommandConfirm` (mounted once, in `App`), then runs
/// again confirmed. Pages that hold their own confirm (page actions, form
/// pages) keep it inline; this is for runs with nowhere to show it.
import { runCommand, undoCommand } from "./api.js";
import { runner, type Runner } from "./commands/runForPerson.js";
import { showToast } from "./components/toastStore.js";
import type { CommandOutcome } from "./tauri-bridge/generated/bindings.js";
import { ranMessage, resultUri, undoneMessage } from "./commandToast.js";

export interface PendingCommand {
  label: string;
  command: string;
  input: unknown;
  /** What it asks about beyond itself, one line each (the permissions a
   *  deploy adds). */
  details?: string[];
}

export interface PersonCommandDeps {
  runCommand(name: string, input: unknown, confirmed: boolean): Promise<CommandOutcome>;
  /** Undo the run recorded as `auditId`, as the person (`undo_command`). */
  undo(auditId: number): Promise<unknown>;
  /** A toast; with `undo`, it offers Undo; with `open`, an Open link. */
  toast(message: string, undo?: () => void, open?: () => void): void;
  /** Open the page a result's `uri` names. */
  openPage?(uri: string): void;
  /** The person's runner (`commands/runForPerson.ts`): a failure is
   *  recorded once; with no control to show it beside, it is the toast's. */
  run: Runner["run"];
}

export function createPersonCommands(deps: PersonCommandDeps) {
  let pending: PendingCommand | null = null;
  const listeners = new Set<() => void>();
  const set = (next: PendingCommand | null) => {
    pending = next;
    for (const l of [...listeners]) l();
  };
  /** The run's outcome — `null` when it failed, or waits for the
   *  person's confirmation (truthy exactly when it ran). */
  // Taking a toast's Undo: the run undone, as the person (tsk975).
  const undo = async (label: string, auditId: number, result?: unknown) => {
    const ran = await deps.run(`Undo ${label}`, () => deps.undo(auditId));
    if (ran.ok) deps.toast(undoneMessage(label, result));
  };
  const attempt = async (p: PendingCommand, confirmed: boolean): Promise<CommandOutcome | null> => {
    const ran = await deps.run(p.label, () => deps.runCommand(p.command, p.input, confirmed), { command: p.command });
    if ("failure" in ran) {
      const f = ran.failure;
      set(f.kind === "confirm" && !confirmed ? { label: p.label, command: p.command, input: p.input, details: f.details } : null);
      return null;
    }
    {
      const out = ran.value;
      set(null);
      // A run that came back undoable offers its Undo.
      const auditId = out.inverse && out.audit_id != null ? out.audit_id : null;
      const uri = resultUri(out.result);
      deps.toast(
        ranMessage(p.label, out.result),
        auditId === null ? undefined : () => void undo(p.label, auditId, out.result),
        uri && deps.openPage ? () => deps.openPage?.(uri) : undefined,
      );
      return out;
    }
  };
  return {
    pending: () => pending,
    subscribe(listener: () => void): () => void {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    run: (label: string, command: string, input: unknown) => attempt({ label, command, input }, false),
    confirm: async () => {
      if (pending) await attempt(pending, true);
    },
    cancel: () => set(null),
  };
}

export type PersonCommands = ReturnType<typeof createPersonCommands>;

/** How the app opens a page (set once, by `App`). */
let opener: ((uri: string) => void) | null = null;
export function setPersonCommandsOpener(open: ((uri: string) => void) | null): void {
  opener = open;
}

/** Open the page `uri` names, as a toast's Open link does. */
export function openPersonUri(uri: string): void {
  opener?.(uri);
}

export const personCommands: PersonCommands = createPersonCommands({
  runCommand: (name, input, confirmed) => runCommand(name, input, confirmed),
  // The person's click on Undo is their confirmation of it.
  undo: (auditId) => undoCommand(auditId, true),
  toast: (message, undo, open) =>
    showToast({ message, onUndo: undo, ...(open ? { link: { label: "Open", onClick: open } } : {}) }),
  openPage: openPersonUri,
  run: runner.run,
});
