import { SettingField } from "../SettingField.js";
import { emptyValue, type Schema } from "../settingsModel.js";
import type { ControlProps } from "./types.js";

const DEFAULT_ITEM: Schema = { type: "string" };

/** A list with one control per item, plus remove, move up or down and add. */
export function ArrayField({ info, value, onChange, label, secret }: ControlProps) {
  const { schema, rootSchema } = info;
  const items = Array.isArray(value) ? value : [];
  const itemSchema = schema.items ?? DEFAULT_ITEM;
  /** Swap item `i` with the one before it. */
  const swapWithPrevious = (i: number) => {
    const next = [...items];
    [next[i - 1], next[i]] = [next[i], next[i - 1]];
    onChange(next);
  };
  return (
    <fieldset>
      <legend>{label}</legend>
      {!items.length && <p className="ox-muted">Empty list</p>}
      {items.map((v, i) => (
        <div key={i} className="ox-field">
          <SettingField
            secret={secret}
            rootSchema={rootSchema}
            schema={itemSchema}
            value={v}
            label={`${label} ${i + 1}`}
            onChange={(next) => onChange(items.map((old, n) => (n === i ? next : old)))}
          />
          <button
            type="button"
            className="ox-button"
            onClick={() => onChange(items.filter((_, n) => n !== i))}
          >
            Remove item {i + 1}
          </button>
          {i > 0 && (
            <button type="button" className="ox-button" onClick={() => swapWithPrevious(i)}>
              Move up
            </button>
          )}
          {i < items.length - 1 && (
            <button type="button" className="ox-button" onClick={() => swapWithPrevious(i + 1)}>
              Move down
            </button>
          )}
        </div>
      ))}
      <button
        type="button"
        className="ox-button"
        disabled={schema.maxItems !== undefined && items.length >= schema.maxItems}
        onClick={() => onChange([...items, emptyValue(itemSchema, rootSchema)])}
      >
        Add item
      </button>
    </fieldset>
  );
}
