import { same } from "../settingsModel.js";
import type { ControlProps } from "./types.js";

/** A select over the schema's `enum` values. */
export function EnumField({ info, value, onChange, label }: ControlProps) {
  const options = info.schema.enum!;
  return (
    <label>
      {label}
      <select
        className="ox-input"
        aria-label={label}
        value={options.findIndex((v) => same(v, value))}
        onChange={(e) => onChange(options[Number(e.target.value)])}
      >
        <option value={-1} disabled>
          Choose a value
        </option>
        {options.map((v, i) => (
          <option key={i} value={i}>
            {v === null ? "Null" : String(v)}
          </option>
        ))}
      </select>
    </label>
  );
}
