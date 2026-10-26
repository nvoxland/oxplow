/// Where a viz component's forms start and submit: a page file's viz node
/// runs them on its open run (`page_form`, `submit_page_form`,
/// `page_action_form`; `pages/useNodeActions.ts` provides that per node).
/// Outside a page run there is nothing to start them on. A form action's
/// submit is a press: it goes through `LensActionRunnerContext`.
import { createContext, useContext } from "react";

import type { CommandOutcome, FormStart, SqlCell } from "../tauri-bridge/generated/bindings.js";

/** What a form names: the viz (or page) and the params it runs with. */
export interface FormTarget {
  config: { id: string };
  params: Record<string, SqlCell>;
}

export interface FormSource {
  /** A form viz's start: its command, values, fixed fields, choices. */
  start(target: FormTarget, streamId: string | null): Promise<FormStart>;
  /** Submit a form viz as the page for the person. */
  submit(target: FormTarget, input: Record<string, unknown>, streamId: string | null, confirmed: boolean): Promise<CommandOutcome>;
  /** A form action's start, for `row` (none: a whole-lens action). */
  actionStart(target: FormTarget, action: string, row: Record<string, SqlCell> | null, streamId: string | null): Promise<FormStart>;
}

const outside = () => Promise.reject(new Error("This isn't on a page, so its form can't open."));
const outsidePage: FormSource = { start: outside, submit: outside, actionStart: outside };

export const FormSourceContext = createContext<FormSource>(outsidePage);

/** Where this viz's forms start and submit. */
export function useFormSource(): FormSource {
  return useContext(FormSourceContext);
}
