import type { InputHTMLAttributes } from "react";

import { useViewParam } from "../tabs/useViewParam.js";
import type { ToolbarFilter } from "./Toolbar.js";

export interface SearchFilterOptions {
  /** The input's accessible name. */
  label: string;
  placeholder?: string;
  testId?: string;
  /** `useListNav`'s `inputProps`, when the filter drives a list. */
  inputProps?: InputHTMLAttributes<HTMLInputElement>;
}

/**
 * A list's search box as a **view param** (`key` in the tab's address): the
 * `Toolbar.filter` to draw and its text to filter by. Typing replaces the
 * tab's entry, so the search comes back with Back and a restart and is in
 * the link; an empty box leaves the address.
 */
export function useSearchFilter(key: string, options: SearchFilterOptions): ToolbarFilter {
  const [value, onChange] = useViewParam<string>(key, "");
  return { value, onChange, ...options };
}
