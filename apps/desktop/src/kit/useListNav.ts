import { useEffect, useState, type HTMLAttributes, type InputHTMLAttributes, type KeyboardEvent } from "react";

/**
 * The one keyboard contract for a list driven from an input (a filter):
 * ↑/↓ move the cursor, Enter picks the row under it, Escape is the
 * page's (`onEscape`: clear the filter). The pointer moves the cursor too.
 * Spread `inputProps` on the input, `listProps` on the list element and
 * `optionProps(i)` on each row; the input names the cursor's row
 * (`aria-activedescendant`), so focus never leaves it. A list that holds
 * focus itself (no input) spreads `keyProps` on the list instead.
 *
 * The cursor is the list's own and goes nowhere; a pick that is kept (a
 * view selection, `useViewSelection`) comes back as `selected`, and the
 * cursor follows it.
 */
export function useListNav({
  count,
  onPick,
  onEscape,
  selected,
  id,
}: {
  count: number;
  onPick(index: number): void;
  /** Escape: true when it did something (cleared the filter), so the key
   *  goes no further. */
  onEscape?(): boolean | void;
  /** The picked row, when the pick is kept: the cursor starts there and
   *  moves there when it changes. */
  selected?: number | null;
  /** Prefix for the rows' element ids. */
  id: string;
}) {
  const [cursor, setCursor] = useState(selected ?? 0);
  useEffect(() => {
    if (selected !== null && selected !== undefined && selected >= 0) setCursor(selected);
  }, [selected]);
  // A shorter list (a narrower filter) keeps the cursor on a row.
  useEffect(() => {
    if (cursor > 0 && cursor >= count) setCursor(Math.max(0, count - 1));
  }, [count, cursor]);
  const optionId = (i: number) => `${id}-option-${i}`;
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" && count > 0) {
      e.preventDefault();
      setCursor((c) => Math.min(count - 1, c + 1));
    } else if (e.key === "ArrowUp" && count > 0) {
      e.preventDefault();
      setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (count > 0) onPick(cursor);
    } else if (e.key === "Escape" && onEscape?.()) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  const keyProps = {
    onKeyDown,
    "aria-activedescendant": count > 0 ? optionId(cursor) : undefined,
  };
  const inputProps: InputHTMLAttributes<HTMLInputElement> = {
    ...keyProps,
    role: "combobox",
    "aria-expanded": count > 0,
    "aria-controls": `${id}-list`,
  };
  const listProps: HTMLAttributes<HTMLElement> = { id: `${id}-list`, role: "listbox" };
  const optionProps = (i: number) => ({
    id: optionId(i),
    role: "option",
    "aria-selected": i === cursor,
    onMouseEnter: () => setCursor(i),
  });
  return { cursor, setCursor, keyProps, inputProps, listProps, optionProps };
}
