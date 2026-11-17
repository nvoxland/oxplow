import { useId, useState } from "react";
import { SettingField } from "../SettingField.js";
import { emptyValue, type Schema } from "../settingsModel.js";
import type { ControlProps } from "./types.js";

/** Declared properties (add or remove each), extra entries, and a new-entry form. */
export function ObjectField({ info, value, onChange, label, secret }: ControlProps) {
  const { schema, rootSchema } = info;
  const id = useId();
  const [newKey, setNewKey] = useState("");
  const obj = value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
  const properties = schema.properties ?? {};
  const extraSchema: Schema = typeof schema.additionalProperties === "object" ? schema.additionalProperties : { type: "string" };
  const remove = (key: string) => {
    const next = { ...obj };
    delete next[key];
    onChange(next);
  };
  const addEntry = () => {
    onChange(Object.assign(Object.create(null), obj, { [newKey]: emptyValue(extraSchema, rootSchema) }));
    setNewKey("");
  };
  return (
    <fieldset>
      <legend>{label}</legend>
      {Object.entries(properties).map(([key, field]) => (
        <div key={key} className="ox-field">
          {Object.hasOwn(obj, key) ? (
            <>
              <SettingField
                secret={secret}
                rootSchema={rootSchema}
                schema={field}
                value={obj[key]}
                label={field.title ?? key}
                onChange={(v) => onChange({ ...obj, [key]: v })}
              />
              {!schema.required?.includes(key) && (
                <button className="ox-button" type="button" onClick={() => remove(key)}>
                  Remove {field.title ?? key}
                </button>
              )}
            </>
          ) : (
            <button
              className="ox-button"
              type="button"
              onClick={() => onChange({ ...obj, [key]: emptyValue(field, rootSchema) })}
            >
              Add {field.title ?? key}
            </button>
          )}
        </div>
      ))}
      {Object.entries(obj)
        .filter(([k]) => !Object.hasOwn(properties, k))
        .map(([key, v]) => (
          <div key={key}>
            <SettingField
              secret={secret}
              rootSchema={rootSchema}
              schema={extraSchema}
              value={v}
              label={key}
              onChange={(next) => onChange({ ...obj, [key]: next })}
            />
            <button className="ox-button" type="button" onClick={() => remove(key)}>
              Remove {key}
            </button>
          </div>
        ))}
      {schema.additionalProperties !== false && (
        <div>
          <label htmlFor={id}>New entry name</label>
          <input id={id} className="ox-input" value={newKey} onChange={(e) => setNewKey(e.target.value)} />
          <button
            className="ox-button"
            type="button"
            disabled={!newKey.trim() || Object.hasOwn(obj, newKey)}
            onClick={addEntry}
          >
            Add entry
          </button>
        </div>
      )}
    </fieldset>
  );
}
