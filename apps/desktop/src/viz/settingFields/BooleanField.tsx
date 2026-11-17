import type { ControlProps } from "./types.js";

/** A checkbox for a boolean value. */
export function BooleanField({ value, onChange, label }: ControlProps) {
  return (
    <label>
      <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} /> {label}
    </label>
  );
}
