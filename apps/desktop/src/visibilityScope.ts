/// Where something the person placed shows: in one thread, in every thread
/// of its stream, or in the whole project (`VisibilityScope` in
/// `crates/oxplow-db/src/visibility_scope.rs`). Bookmarks use it. Bare
/// "scope" is what a contribution may call on the host.
export type VisibilityScope = "thread" | "stream" | "project";

/** Narrowest first, as a viewer sees them. */
export const VISIBILITY_SCOPES: VisibilityScope[] = ["thread", "stream", "project"];

/** What the person reads for each: "This thread", "This stream", "This project". */
export function visibilityScopeLabel(scope: VisibilityScope): string {
  return scope === "thread" ? "This thread" : scope === "stream" ? "This stream" : "This project";
}
