import { useId, useState } from "react";

import type { CommandOutcome } from "../tauri-bridge/generated/bindings.js";
import { showToast } from "../components/toastStore.js";
import { useActionAnchor, useRunner } from "../commands/RunnerContext.js";
import { undoCommand } from "../api.js";
import { ranMessage, resultUri, undoneMessage } from "../commandToast.js";
import { openPersonUri } from "../personCommands.js";

/** A form submitted, as `FormViz` and `ActionForm` both submit one,
 *  through the person's runner (`commands/runForPerson.ts`): the person's
 *  input runs; a command that asks is held for them to confirm; an invalid
 *  field's message goes under that field; anything else is recorded once
 *  and is `failure`, shown beside the submit. `anchor`: the key it shows by
 *  (default: this form's own). */
export function useFormSubmit(
  label: string,
  run: (input: Record<string, unknown>, confirmed: boolean) => Promise<CommandOutcome>,
  onDone: (outcome: CommandOutcome) => void,
  anchor?: string,
) {
  const own = useId();
  const key = anchor ?? `form/${own}`;
  const failure = useActionAnchor(key);
  const runner = useRunner();
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<Record<string, unknown> | null>(null);
  // What a command that asks asks about beyond itself, one line each.
  const [pendingDetails, setPendingDetails] = useState<string[]>([]);
  const [serverErrors, setServerErrors] = useState<Record<string, string> | undefined>(undefined);
  const submit = async (input: Record<string, unknown>, confirmed: boolean) => {
    setBusy(true);
    try {
      const ran = await runner.run(label, () => run(input, confirmed), { anchor: key, showsFields: true });
      if ("failure" in ran) {
        const f = ran.failure;
        if (f.kind === "confirm" && !confirmed) {
          setPendingDetails(f.details);
          setPending(input);
        }
        else if (f.kind === "field") setServerErrors(f.errors);
        return;
      }
      const outcome = ran.value;
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
                void runner
                  .run(`Undo ${label}`, () => undoCommand(auditId, true))
                  .then((r) => (r.ok ? showToast({ message: undoneMessage(label, outcome.result) }) : undefined)),
            }),
      });
      onDone(outcome);
    } finally {
      setBusy(false);
    }
  };
  return { busy, pending, pendingDetails, serverErrors, failure, submit, cancel: () => setPending(null) };
}
