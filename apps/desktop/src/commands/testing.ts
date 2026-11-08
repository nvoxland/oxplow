/// A runner for tests: what it records goes to `onReport`, and nothing
/// reaches the daemon.
import type { OpErrorInput } from "../opErrors.js";
import { createFailureStore, createInlineClaims } from "./actionFailures.js";
import { createRunner, type Runner } from "./runForPerson.js";

export function recordingRunner(onReport: (entry: OpErrorInput) => void = () => {}): Runner {
  return createRunner({
    report: async (entry) => {
      onReport(entry);
      return null;
    },
    failures: createFailureStore(),
    claims: createInlineClaims(),
    now: () => 0,
  });
}
