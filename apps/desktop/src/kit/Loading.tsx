/** The one "still reading" line: `Reading <what>…`. */
export function Loading({ what }: { what: string }) {
  return <p className="ox-loading">Reading {what}…</p>;
}

/** A whole page still on its way: a spinner in the middle of the page. A part
 *  of a page uses [`Loading`]'s one line. */
export function LoadingScreen() {
  return (
    <div className="ox-loading-screen" role="status" data-testid="loading-screen">
      <span className="ox-spinner" aria-hidden />
      <span>Loading…</span>
    </div>
  );
}
