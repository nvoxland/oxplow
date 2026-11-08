import type { InlineFailure } from "../commands/actionFailures.js";
import { alertsRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";

/** A person's command that failed, beside the control that ran it: its
 *  first line (the whole message is the hover), and Details, the Alerts
 *  page, where it is recorded. */
export function InlineError({ failure, testId = "inline-error" }: { failure: InlineFailure; testId?: string }) {
  const first = failure.message.split("\n")[0]!;
  return (
    <span className="ox-error" role="alert" data-testid={testId} title={failure.message} style={{ display: "inline-flex", gap: 6, alignItems: "baseline" }}>
      <span>{first}</span>
      <RouteLink to={alertsRef().id} testId="inline-error-details" className="ox-link">
        Details
      </RouteLink>
    </span>
  );
}
