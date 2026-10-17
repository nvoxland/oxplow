/** The one "still reading" line: `Reading <what>…`. */
export function Loading({ what }: { what: string }) {
  return <p className="ox-loading">Reading {what}…</p>;
}
