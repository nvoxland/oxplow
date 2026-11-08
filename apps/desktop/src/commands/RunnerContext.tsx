import { createContext, useContext, useEffect, useSyncExternalStore } from "react";

import type { InlineFailure } from "./actionFailures.js";
import { runner, type Runner } from "./runForPerson.js";

/** The runner below it (a test's); none: the window's. */
export const RunnerContext = createContext<Runner>(runner);

export function useRunner(): Runner {
  return useContext(RunnerContext);
}

/** A control that runs a person's command, keyed `key`: while it is on
 *  screen its failures show beside it (and aren't toasted). The last
 *  failure, or null. The key outlives the control, so one drawn again (a
 *  list re-read) still shows it. */
export function useActionAnchor(key: string): InlineFailure | null {
  const { failures } = useRunner();
  useEffect(() => failures.mount(key), [failures, key]);
  return useSyncExternalStore(failures.subscribe, () => failures.get(key));
}
