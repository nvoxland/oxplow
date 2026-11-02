import { useState } from "react";

import type { CommandOutcome } from "../tauri-bridge/generated/bindings.js";
import { showToast } from "../components/toastStore.js";
import { fieldErrors, needsConfirmation } from "../ipc-error.js";
import { recordOpError } from "../opErrors.js";
import { undoCommand } from "../api.js";
import { ranMessage, resultUri, undoneMessage } from "../commandToast.js";
import { openPersonUri } from "../personCommands.js";

/** A form submitted, as `FormViz` and `ActionForm` both submit one: the
 *  person's input runs; a command that asks is held for them to confirm;
 *  an invalid field's message goes under that field; anything else is a
 *  recorded failure. */
export function useFormSubmit(
  label: string,
  run: (input: Record<string, unknown>, confirmed: boolean) => Promise<CommandOutcome>,
  onDone: (outcome: CommandOutcome) => void,
) {
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<Record<string, unknown> | null>(null);
  const [serverErrors, setServerErrors] = useState<Record<string, string> | undefined>(undefined);
  const submit = async (input: Record<string, unknown>, confirmed: boolean) => {
    setBusy(true);
    try {
      const outcome = await run(input, confirmed);
      setPending(null);
      setServerErrors(undefined);
      // A run that came back undoable offers its Undo, as a pressed action's
      // does (the person's click on Undo is their confirmation of it).
      const auditId = outcome.inverse && outcome.audit_id != null ? outcome.audit_id : null;
      showToast({
        message: ranMessage(label, outcome.result),
        ...(resultUri(outcome.result) === null
          ? {}
          : {
              link: {
                label: "Open",
                onClick: () => {
                  const uri = resultUri(outcome.result);
                  if (uri) openPersonUri(uri);
                },
              },
            }),
        ...(auditId === null
          ? {}
          : {
              onUndo: () =>
                void undoCommand(auditId, true)
                  .then(() => showToast({ message: undoneMessage(label, outcome.result) }))
                  .catch((e: unknown) =>
                    recordOpError({ label: `Undo ${label}`, message: e instanceof Error ? e.message : String(e) }),
                  ),
            }),
      });
      onDone(outcome);
    } catch (e) {
      const fields = fieldErrors(e);
      if (!confirmed && needsConfirmation(e)) setPending(input);
      else if (fields) setServerErrors(fields);
      else recordOpError({ label, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };
  return { busy, pending, serverErrors, submit, cancel: () => setPending(null) };
}

