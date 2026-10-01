/// Commands a person runs from outside a page (P6.D1: a launcher entry).
/// A command that asks for confirmation waits here until the person
/// answers in `PersonCommandConfirm` (mounted once, in `App`), then runs
/// again confirmed. Pages that hold their own confirm (lens actions, form
/// lenses) keep it inline; this is for runs with nowhere to show it.
import { runCommand } from "./api.js";
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
  toast(message: string): void;
  recordError(label: string, message: string): void;
}

export function createPersonCommands(deps: PersonCommandDeps) {
  let pending: PendingCommand | null = null;
  const listeners = new Set<() => void>();
  const set = (next: PendingCommand | null) => {
    pending = next;
    for (const l of [...listeners]) l();
  };
  /** Whether the command ran (false when it failed, or waits for the
   *  person's confirmation). */
  const attempt = async (p: PendingCommand, confirmed: boolean): Promise<boolean> => {
    try {
      await deps.runCommand(p.command, p.input, confirmed);
      set(null);
      deps.toast(`${p.label}: done.`);
      return true;
    } catch (e) {
      if (!confirmed && needsConfirmation(e)) {
        set(p);
        return false;
      }
      set(null);
      deps.recordError(p.label, e instanceof Error ? e.message : String(e));
      return false;
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
  toast: (message) => showToast({ message }),
  recordError: (label, message) => recordOpError({ label, message }),
});
