import { useId } from "react";
import { Markdown } from "../../components/Markdown/Markdown.js";
import type { ControlProps } from "./types.js";

/** Markdown editor, textarea, password, number or plain text input. */
export function TextField({ info, value, onChange, label, secret = false, multiline = false }: ControlProps) {
  const { schema, type } = info;
  const id = useId();
  if (schema.contentMediaType === "text/markdown" && !secret) {
    return (
      <div>
        <span>{label}</span>
        <Markdown value={String(value ?? "")} onChange={onChange} />
      </div>
    );
  }
  const numeric = type === "number" || type === "integer";
  const inputType = secret ? "password" : numeric ? "number" : "text";
  const parse = (text: string): unknown => (numeric ? (text === "" ? undefined : Number(text)) : text);
  return (
    <label htmlFor={id} className="ox-field">
      {label}
      {multiline && !secret && !numeric ? (
        <textarea
          id={id}
          className="ox-input"
          rows={5}
          value={String(value ?? "")}
          onChange={(e) => onChange(e.target.value)}
        />
      ) : (
        <input
          id={id}
          className="ox-input"
          type={inputType}
          autoComplete={secret ? "new-password" : undefined}
          min={schema.minimum}
          max={schema.maximum}
          step={type === "integer" ? 1 : "any"}
          value={typeof value === "number" && !Number.isFinite(value) ? "" : String(value ?? "")}
          onChange={(e) => onChange(parse(e.target.value))}
        />
      )}
    </label>
  );
}
