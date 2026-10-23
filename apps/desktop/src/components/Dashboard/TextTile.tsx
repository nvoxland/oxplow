import { InlineEdit } from "../InlineEdit.js";

/** The heading's look — one line, left-aligned, sized between the page h1 and a
 *  tile title so it reads as a divider between groups of tiles. */
const HEADING_STYLE: React.CSSProperties = {
  display: "block",
  textAlign: "left",
  fontSize: 15,
  fontWeight: 600,
  lineHeight: 1.4,
  color: "var(--text, #ddd)",
};

/** A heading band's text: edited in place with `InlineEdit` when it can be
 *  saved (`onCommit`), else plain. The custom-dashboard page's `TextTile`
 *  and a dashboard lens's text row tiles draw it. */
export function TextBand({ text, onCommit, testId }: { text: string; onCommit?: (next: string) => void; testId: string }) {
  if (!onCommit) {
    return (
      <span data-testid={testId} style={HEADING_STYLE}>
        {text}
      </span>
    );
  }
  return (
    <InlineEdit
      value={text}
      onCommit={onCommit}
      allowEmpty
      placeholder="Click to add a heading…"
      ariaLabel="Section heading"
      testId={testId}
      displayStyle={HEADING_STYLE}
      inputStyle={{ ...HEADING_STYLE, width: "100%" }}
    />
  );
}

/** A heading band's frame: no card, only a rule under the text. */
export const BAND_STYLE: React.CSSProperties = {
  // A band, not a panel: no card fill or border, just a rule under the
  // heading, and only as tall as the text itself.
  borderBottom: "1px solid var(--border-subtle)",
  padding: "4px 2px 6px",
  minWidth: 0,
  alignSelf: "start",
};
