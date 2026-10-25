/// Where a viz component's forms start and submit. A lens's form runs as
/// the lens (`lens_form`, `submit_lens_form`, `lens_action_form`: the
/// default); a page file's viz node runs on its open run (`page_form`,
/// `submit_page_form`, `page_action_form`; `pages/useNodeActions.ts`
/// provides that), so the same form components draw both. A form action's
/// submit is a press: it goes through `LensActionRunnerContext`.
import { createContext, useContext } from "react";

import { lensActionForm, lensForm, submitLensForm } from "../api.js";
import type { CommandOutcome, FormStart, SqlCell } from "../tauri-bridge/generated/bindings.js";

/** What a form names: the lens (or node) and the params it runs with. */
export interface FormTarget {
  lens: { id: string };
  params: Record<string, SqlCell>;
}

export interface FormSource {
  /** A form viz's start: its command, values, fixed fields, choices. */
  start(target: FormTarget, streamId: string | null): Promise<FormStart>;
  /** Submit a form viz as the lens (or page) for the person. */
  submit(target: FormTarget, input: Record<string, unknown>, streamId: string | null, confirmed: boolean): Promise<CommandOutcome>;
  /** A form action's start, for `row` (none: a whole-lens action). */
  actionStart(target: FormTarget, action: string, row: Record<string, SqlCell> | null, streamId: string | null): Promise<FormStart>;
}

// Called through, so a test's `mock.module("../api.js")` is what runs.
const viaLens: FormSource = {
  start: (t, streamId) => lensForm(t.lens.id, t.params, streamId),
  submit: (t, input, streamId, confirmed) => submitLensForm(t.lens.id, input, t.params, streamId, confirmed),
  actionStart: (t, action, row, streamId) => lensActionForm(t.lens.id, action, t.params, row, streamId),
};

export const FormSourceContext = createContext<FormSource>(viaLens);

/** Where this viz's forms start and submit. */
export function useFormSource(): FormSource {
  return useContext(FormSourceContext);
}
