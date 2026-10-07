/// Commands a person runs from outside a page (P6.D1: a launcher entry).
/// A command that asks for confirmation waits here until the person
/// answers in `PersonCommandConfirm` (mounted once, in `App`), then runs
/// again confirmed. Pages that hold their own confirm (lens actions, form
/// lenses) keep it inline; this is for runs with nowhere to show it.
import { runCommand, undoCommand } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { showToast } from "./components/toastStore.js";
import { needsConfirmation } from "./ipc-error.js";
import type { CommandOutcome } from "./tauri-bridge/generated/bindings.js";

export interface PendingCommand {
  label: string;
  command: string;
  input: unknown;
}

export interface PersonCommandDeps {
  runCommand(name: string, input: unknown, confirmed: boolean): Promise<CommandOutcome>;
  /** Undo the run recorded as `auditId`, as the person (`undo_command`). */
  undo(auditId: number): Promise<unknown>;
  /** A toast; with `undo`, it offers Undo. */
  toast(message: string, undo?: () => void): void;
  recordError(label: string, message: string): void;
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
  const undo = async (label: string, auditId: number) => {
    try {
      await deps.undo(auditId);
      deps.toast(`${label}: undone.`);
    } catch (e) {
      deps.recordError(`Undo ${label}`, e instanceof Error ? e.message : String(e));
    }
  };
  const attempt = async (p: PendingCommand, confirmed: boolean): Promise<CommandOutcome | null> => {
    try {
      const out = await deps.runCommand(p.command, p.input, confirmed);
      set(null);
      // A run that came back undoable offers its Undo.
      const auditId = out.inverse && out.audit_id != null ? out.audit_id : null;
      deps.toast(`${p.label}: done.`, auditId === null ? undefined : () => void undo(p.label, auditId));
      return out;
    } catch (e) {
      if (!confirmed && needsConfirmation(e)) {
        set(p);
        return null;
      }
      set(null);
      deps.recordError(p.label, e instanceof Error ? e.message : String(e));
      return null;
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

export const personCommands: PersonCommands = createPersonCommands({
  runCommand: (name, input, confirmed) => runCommand(name, input, confirmed),
  // The person's click on Undo is their confirmation of it.
  undo: (auditId) => undoCommand(auditId, true),
  toast: (message, undo) => showToast({ message, onUndo: undo }),
  recordError: (label, message) => recordOpError({ label, message }),
});
