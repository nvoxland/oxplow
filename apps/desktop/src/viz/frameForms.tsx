/// The forms a component's frame asks the host to show: a declared
/// command's (`openForm`, run as the component) and one of its own schema
/// (`openForm({ schema })`, answering the values submitted). Shared by every
/// host that draws a slideover for a frame — a custom page and a viz's own
/// component (a settings section, a tile).
import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";

import { componentForm, invokeComponentCommand } from "../api.js";
import { SchemaForm } from "../components/SchemaForm/SchemaForm.js";
import { TabModal } from "../components/TabModal.js";
import { recordOpError } from "../opErrors.js";
import type { FormStart } from "../tauri-bridge/generated/bindings.js";
import type { FormDeps } from "./ComponentFrame.js";
import type { OwnForm } from "./componentBridge.js";
import { FormModal } from "./VizView.js";
import { useFormSubmit } from "./useFormSubmit.js";

interface AskedForm {
  command: string;
  input: unknown;
  version: string;
  resolve(out: { result: unknown } | null): void;
  reject(e: unknown): void;
}

/** The form deps a frame takes, and the slideovers to render beside it
 *  while one is open. `title` labels the confirm. */
export function useFrameForms(componentKey: string | null, title: string): { forms: FormDeps; element: ReactNode } {
  const [asked, setAsked] = useState<AskedForm | null>(null);
  // A form of the frame's own schema, shown while it waits for its values.
  const [ownAsked, setOwnAsked] = useState<{ form: OwnForm; resolve(values: Record<string, unknown> | null): void } | null>(null);
  const forms = useMemo<FormDeps>(
    () => ({
      openForm: (command, input, version) =>
        new Promise<{ result: unknown } | null>((resolve, reject) => {
          setAsked({ command, input, version, resolve, reject });
        }),
      // A newer form closes the one open: that one's frame hears none.
      askForm: (form) =>
        new Promise<Record<string, unknown> | null>((resolve) => {
          setOwnAsked((open) => {
            open?.resolve(null);
            return { form, resolve };
          });
        }),
    }),
    [],
  );
  const element = (
    <>
      {ownAsked ? (
        <TabModal
          onClose={() => {
            ownAsked.resolve(null);
            setOwnAsked(null);
          }}
          title={ownAsked.form.title}
          testId="custom-page-own-form"
        >
          <SchemaForm
            schema={ownAsked.form.schema}
            initial={ownAsked.form.values}
            submitLabel="Submit"
            onSubmit={(values) => {
              ownAsked.resolve(values);
              setOwnAsked(null);
            }}
            testIdPrefix="custom-page-own-form-fields"
          />
        </TabModal>
      ) : null}
      {asked && componentKey ? <CommandForm componentKey={componentKey} title={title} asked={asked} onDone={() => setAsked(null)} /> : null}
    </>
  );
  return { forms, element };
}

/** A declared command's form the frame asked for, in a slideover: its
 *  fields start from the frame's input; submitting runs it as the
 *  component (the confirm, when it asks, is the host's) and answers the
 *  frame with its result; closing it answers none. */
function CommandForm({
  componentKey,
  title,
  asked,
  onDone,
}: {
  componentKey: string;
  title: string;
  asked: AskedForm;
  onDone(): void;
}) {
  const [start, setStart] = useState<FormStart | null>(null);
  const answered = useRef(false);
  const answer = (out: { result: unknown } | null) => {
    if (answered.current) return;
    answered.current = true;
    asked.resolve(out);
    onDone();
  };
  const form = useFormSubmit(
    title,
    (input, confirmed) => invokeComponentCommand(componentKey, asked.command, input, confirmed, asked.version),
    (outcome) => answer({ result: outcome.result }),
  );
  useEffect(() => {
    let live = true;
    componentForm(componentKey, asked.command, asked.input, asked.version).then(
      (s) => {
        if (live) setStart(s);
      },
      (e: unknown) => {
        if (!live) return;
        recordOpError({ label: title, message: e instanceof Error ? e.message : String(e) });
        answered.current = true;
        asked.reject(e);
        onDone();
      },
    );
    return () => {
      live = false;
    };
    // The form asked for is `asked`; it is fixed for this mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [asked]);
  return (
    <FormModal
      title={title}
      start={start}
      form={form}
      submitLabel="Submit"
      onClose={() => answer(null)}
      testId="custom-page-form"
    />
  );
}

