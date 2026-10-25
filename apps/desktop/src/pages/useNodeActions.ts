/// A page file's actions (`.context/pages-and-tabs.md` "Page files",
/// Actions): pressed on its open run with `run_page_action`. A viz node's
/// row and whole-lens actions come from the lens viz components, which
/// press through `LensActionRunnerContext`; `nodeActionRunner` is that
/// runner for one node, so the components draw a page's nodes unchanged.
import { createContext, useContext, useMemo } from "react";

import { unwrapResult } from "../api.js";
import type { FormSource } from "../lens/formSource.js";
import type { LensActionRunner } from "../lens/lensActionRunner.js";
import { commands, type CommandOutcome, type Drop, type FormStart, type SqlCell } from "../tauri-bridge/generated/bindings.js";

/** `commands.runPageAction`, unwrapped. */
export type RunPageAction = (
  runId: string,
  node: string | null,
  action: string,
  row: Record<string, SqlCell> | null,
  input: unknown,
  drop: Drop | null,
  confirmed: boolean,
) => Promise<CommandOutcome>;

const daemonRunPageAction: RunPageAction = async (runId, node, action, row, input, drop, confirmed) =>
  unwrapResult(await commands.runPageAction(runId, node, action, row, input ?? null, drop, confirmed));

/** How a page host presses its actions (the daemon; a test's fake). */
export const PageActionContext = createContext<RunPageAction>(daemonRunPageAction);

/** The runner for node `node` (an embed's `<embed>/<id>`; null: the
 *  page's own actions) of open run `runId`. The lens id and params a viz
 *  passes are the run's own, which the server holds. */
export function nodeActionRunner(press: RunPageAction, runId: string | null, node: string | null): LensActionRunner {
  return (_lens, action, _params, row, _streamId, confirmed, input = null, drop = null) => {
    if (!runId) return Promise.reject(new Error("The page isn't open yet; try again in a moment."));
    return press(runId, node, action, row, input, drop, confirmed);
  };
}

/** `nodeActionRunner` over the page's action context. */
export function useNodeActions(runId: string | null, node: string | null): LensActionRunner {
  const press = useContext(PageActionContext);
  return useMemo(() => nodeActionRunner(press, runId, node), [press, runId, node]);
}

// --- Forms ---------------------------------------------------------------

/** A page's forms on its open run: a form node's start and submit, and a
 *  form action's start (its submit is a press, `run_page_action` with the
 *  input). `node` is the node's path; null a whole-page action. */
export interface PageForms {
  start(runId: string, node: string): Promise<FormStart>;
  submit(runId: string, node: string, input: unknown, confirmed: boolean): Promise<CommandOutcome>;
  actionStart(runId: string, node: string | null, action: string, row: Record<string, SqlCell> | null): Promise<FormStart>;
}

const daemonForms: PageForms = {
  start: async (runId, node) => unwrapResult(await commands.pageForm(runId, node)) as FormStart,
  submit: async (runId, node, input, confirmed) => unwrapResult(await commands.submitPageForm(runId, node, input ?? null, confirmed)),
  actionStart: async (runId, node, action, row) => unwrapResult(await commands.pageActionForm(runId, node, action, row)) as FormStart,
};

/** How a page host reads and submits its forms (the daemon; a test's fake). */
export const PageFormsContext = createContext<PageForms>(daemonForms);

const notOpen = () => Promise.reject(new Error("The page isn't open yet; try again in a moment."));

/** The form source for node `node` (null: the page's own actions) of open
 *  run `runId`: the lens id and params a form passes are the run's own,
 *  which the server holds. A form node needs a node. */
export function nodeFormSource(forms: PageForms, runId: string | null, node: string | null): FormSource {
  return {
    start: () => (runId && node ? forms.start(runId, node) : notOpen()),
    submit: (_t, input, _s, confirmed) => (runId && node ? forms.submit(runId, node, input, confirmed) : notOpen()),
    actionStart: (_t, action, row) => (runId ? forms.actionStart(runId, node, action, row) : notOpen()),
  };
}

/** `nodeFormSource` over the page's forms context. */
export function useNodeForms(runId: string | null, node: string | null): FormSource {
  const forms = useContext(PageFormsContext);
  return useMemo(() => nodeFormSource(forms, runId, node), [forms, runId, node]);
}
