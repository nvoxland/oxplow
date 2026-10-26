/// How a viz component's actions reach the server. A viz is always drawn
/// inside a page run: `pages/useNodeActions.ts` provides, per viz node, a
/// runner that presses on the open run (`run_page_action`). Outside one
/// there is nothing to press on.
import { createContext, useContext } from "react";

import type { CommandOutcome, Drop, SqlCell } from "../tauri-bridge/generated/bindings.js";

/** What a viz component presses with: the viz (only its id, which the run
 *  holds) and its params, the action, the row it is on, and for a form
 *  action its input, for a drag action where the row landed. */
export type VizActionRunner = (
  id: string,
  action: string,
  params: Record<string, SqlCell>,
  row: Record<string, SqlCell> | null,
  streamId: string | null,
  confirmed: boolean,
  input?: unknown,
  drop?: Drop | null,
) => Promise<CommandOutcome>;

const outsidePage: VizActionRunner = () => Promise.reject(new Error("This isn't on a page, so its actions can't run."));

export const VizActionRunnerContext = createContext<VizActionRunner>(outsidePage);

/** What this viz's actions run through. */
export function useVizActionRunner(): VizActionRunner {
  return useContext(VizActionRunnerContext);
}
