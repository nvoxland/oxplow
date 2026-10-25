/// How a viz component's actions reach the server. A lens page runs them
/// as the lens (`run_lens_action`, the default); a page file's viz node
/// runs them on its open run (`run_page_action`, `pages/useNodeActions.ts`
/// provides that), so the same viz components draw both.
import { createContext, useContext } from "react";

import { runLensAction } from "../api.js";

/** `runLensAction`'s shape: what a viz component presses with. */
export type LensActionRunner = typeof runLensAction;

// Called through, so a test's `mock.module("../api.js")` is what runs.
const viaLens: LensActionRunner = (...args) => runLensAction(...args);

export const LensActionRunnerContext = createContext<LensActionRunner>(viaLens);

/** What this viz's actions run through. */
export function useLensActionRunner(): LensActionRunner {
  return useContext(LensActionRunnerContext);
}
