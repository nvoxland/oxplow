import { SettingField } from "../SettingField.js";
import { emptyValue } from "../settingsModel.js";
import type { ControlProps } from "./types.js";

/** A null checkbox, and the control for the non-null value while it is unchecked. */
export function NullableField({ info, value, onChange, label, secret, multiline }: ControlProps) {
  const { schema, resolved, branches, type, rootSchema } = info;
  const inner = branches.length > 1
    ? { ...resolved, type, anyOf: branches, oneOf: undefined }
    : { ...schema, type, anyOf: undefined, oneOf: undefined };
  return (
    <div>
      <label>
        <input
          type="checkbox"
          checked={value === null}
          onChange={(e) => onChange(e.target.checked ? null : emptyValue(schema, rootSchema))}
        />{" "}
        Null
      </label>
      {value !== null && (
        <SettingField
          secret={secret}
          rootSchema={rootSchema}
          schema={inner}
          value={value}
          onChange={onChange}
          label={label}
          multiline={multiline}
        />
      )}
    </div>
  );
}
