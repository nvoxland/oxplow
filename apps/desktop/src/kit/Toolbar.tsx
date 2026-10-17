import type { InputHTMLAttributes } from "react";

import { ActionButtons } from "./ActionButtons.js";
import type { PageAction } from "./actions.js";

export interface ToolbarFilter {
  value: string;
  onChange(value: string): void;
  /** The input's accessible name. */
  label: string;
  placeholder?: string;
  testId?: string;
  /** `useListNav`'s `inputProps`, when the filter drives a list. */
  inputProps?: InputHTMLAttributes<HTMLInputElement>;
}

/** The row above a list: its filter, then its actions. */
export function Toolbar({ filter, actions }: { filter?: ToolbarFilter; actions?: readonly PageAction[] }) {
  return (
    <div className="ox-toolbar">
      {filter ? (
        <input
          type="text"
          className="ox-input"
          data-testid={filter.testId}
          aria-label={filter.label}
          placeholder={filter.placeholder}
          value={filter.value}
          onChange={(e) => filter.onChange(e.target.value)}
          {...filter.inputProps}
        />
      ) : null}
      {actions ? <ActionButtons actions={actions} /> : null}
    </div>
  );
}
