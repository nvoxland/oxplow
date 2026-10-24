import type { Extension } from "./api.js";

export const FOUNDATION = "oxplow-foundation";
/** Deploys every enabled extension whose source changed, foundation first. */
export const RECOVERY_COMMAND = "cargo run -q -p oxplow-dev -- extension deploy";

/**
 * Why the required foundation extension isn't running, as lines to show,
 * or `null` while the listing is loading or foundation is enabled. An
 * empty listing is one that failed to load (`extensionsStore` settles a
 * failure as `[]`; the error is reported there), not a missing foundation.
 */
export function foundationDown(extensions: Extension[] | null): string[] | null {
  if (!extensions || extensions.length === 0) return null;
  const entry = extensions.find((e) => e.name === FOUNDATION);
  if (!entry) return [`${FOUNDATION} isn't in the extension listing.`];
  if (entry.enabled) return null;
  return entry.errors.length > 0 ? entry.errors : [`${FOUNDATION} is held back, with no reason given.`];
}
