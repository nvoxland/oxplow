/// "Open with": the pages that render the shown URI, when there's more
/// than one, and which of them draws it now. Picking one writes the URI's
/// view (`#oxplow.renderer=<extension>/<page>`), so the same URI stays in
/// the tab and Back returns to how it was shown.
import type { CSSProperties } from "react";

export interface OpenWithProps {
  /** The renderers, by name (`<extension>/<page>`), with their titles. */
  options: Array<{ name: string; title: string }>;
  /** The one drawing the page now. */
  current: string;
  pick(name: string): void;
  style?: CSSProperties;
}

export function OpenWith({ options, current, pick, style }: OpenWithProps) {
  if (options.length < 2) return null;
  return (
    <select
      data-testid="page-nav-open-with"
      aria-label="Open with"
      title="Open with"
      value={current}
      onChange={(e) => pick(e.target.value)}
      style={style}
    >
      {options.map((o) => (
        <option key={o.name} value={o.name}>
          {o.title}
        </option>
      ))}
    </select>
  );
}
