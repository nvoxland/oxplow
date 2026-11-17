import { SettingField } from "../SettingField.js";
import { emptyValue, validate } from "../settingsModel.js";
import type { ControlProps } from "./types.js";

/** A format picker for oneOf/anyOf schemas, and the control for the chosen branch. */
export function AlternativesField({ info, value, onChange, label, secret, multiline }: ControlProps) {
  const { branches, rootSchema } = info;
  const exact = branches.findIndex((s) => !validate(s, value, "Value", rootSchema).length);
  const actualType = value === null ? "null" : Array.isArray(value) ? "array" : typeof value;
  const typeMatch = branches.findIndex(
    (s) => s.type === actualType || (s.type === "integer" && actualType === "number"),
  );
  const selected = exact >= 0 ? exact : Math.max(0, typeMatch);
  return (
    <div>
      <label>
        {label} format
        <select
          className="ox-input"
          aria-label={`${label} format`}
          value={selected}
          onChange={(e) => onChange(emptyValue(branches[Number(e.target.value)]!, rootSchema))}
        >
          {branches.map((s, i) => (
            <option key={i} value={i}>
              {s.title ?? (typeof s.type === "string" ? s.type : `Option ${i + 1}`)}
            </option>
          ))}
        </select>
      </label>
      <SettingField
        secret={secret}
        rootSchema={rootSchema}
        schema={branches[selected]!}
        value={value}
        onChange={onChange}
        label={label}
        multiline={multiline}
      />
    </div>
  );
}
