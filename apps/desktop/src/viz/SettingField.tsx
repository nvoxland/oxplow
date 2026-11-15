import { useId, useState } from "react";
import { Markdown } from "../components/Markdown/Markdown.js";
import { alternatives, concreteSchema, emptyValue, resolveSchema, same, validate, type Schema } from "./settingsModel.js";

/** Structured controls retain JSON types; collections are edited as one value. */
export function SettingField({ schema: original, value, onChange, label, secret = false, multiline = false, rootSchema = original }: { schema: Schema; value: unknown; onChange(value: unknown): void; label: string; secret?: boolean; multiline?: boolean; rootSchema?: Schema }) {
  const id = useId();
  const [newKey, setNewKey] = useState("");
  const schema = concreteSchema(original, rootSchema);
  if ("const" in schema) return <p>{label}: {String((schema as { const: unknown }).const)} (fixed)</p>;
  const resolved = resolveSchema(original, rootSchema);
  const branches = alternatives(resolved, rootSchema).filter((s) => s.type !== "null");
  const nullable = (Array.isArray(resolved.type) && resolved.type.includes("null")) || alternatives(resolved, rootSchema).some((s) => s.type === "null");
  const type = Array.isArray(schema.type) ? schema.type.find((t) => t !== "null") : schema.type;
  if (nullable) return <div><label><input type="checkbox" checked={value === null} onChange={(e) => onChange(e.target.checked ? null : emptyValue(schema, rootSchema))} /> Null</label>{value !== null && <SettingField secret={secret} rootSchema={rootSchema} schema={branches.length > 1 ? { ...resolved, type, anyOf: branches, oneOf: undefined } : { ...schema, type, anyOf: undefined, oneOf: undefined }} value={value} onChange={onChange} label={label} multiline={multiline} />}</div>;
  if (branches.length > 1) {
    const exact = branches.findIndex((s) => !validate(s, value, "Value", rootSchema).length);
    const actualType = value === null ? "null" : Array.isArray(value) ? "array" : typeof value;
    const typeMatch = branches.findIndex((s) => s.type === actualType || s.type === "integer" && actualType === "number");
    const selected = exact >= 0 ? exact : Math.max(0, typeMatch);
    return <div><label>{label} format<select className="ox-input" aria-label={`${label} format`} value={selected} onChange={(e) => onChange(emptyValue(branches[Number(e.target.value)]!, rootSchema))}>{branches.map((s, i) => <option key={i} value={i}>{s.title ?? (typeof s.type === "string" ? s.type : `Option ${i + 1}`)}</option>)}</select></label><SettingField secret={secret} rootSchema={rootSchema} schema={branches[selected]!} value={value} onChange={onChange} label={label} multiline={multiline} /></div>;
  }
  if (schema.enum) return <label>{label}<select className="ox-input" aria-label={label} value={schema.enum.findIndex((v) => same(v, value))} onChange={(e) => onChange(schema.enum![Number(e.target.value)])}><option value={-1} disabled>Choose a value</option>{schema.enum.map((v, i) => <option key={i} value={i}>{v === null ? "Null" : String(v)}</option>)}</select></label>;
  if (type === "boolean") return <label><input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} /> {label}</label>;
  if (type === "array") {
    const items = Array.isArray(value) ? value : [];
    return <fieldset><legend>{label}</legend>{!items.length && <p className="ox-muted">Empty list</p>}{items.map((v, i) => <div key={i} style={{ marginBottom: 10 }}><SettingField secret={secret} rootSchema={rootSchema} schema={schema.items ?? { type: "string" }} value={v} label={`${label} ${i + 1}`} onChange={(next) => onChange(items.map((old, n) => n === i ? next : old))} /><button type="button" className="ox-button" onClick={() => onChange(items.filter((_, n) => n !== i))}>Remove item {i + 1}</button>{i > 0 && <button type="button" className="ox-button" onClick={() => { const next = [...items]; [next[i - 1], next[i]] = [next[i], next[i - 1]]; onChange(next); }}>Move up</button>}</div>)}<button type="button" className="ox-button" disabled={schema.maxItems !== undefined && items.length >= schema.maxItems} onClick={() => onChange([...items, emptyValue(schema.items ?? { type: "string" }, rootSchema)])}>Add item</button></fieldset>;
  }
  if (type === "object" || schema.properties) {
    const obj = value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
    const properties = schema.properties ?? {};
    const remove = (key: string) => { const next = { ...obj }; delete next[key]; onChange(next); };
    return <fieldset><legend>{label}</legend>{Object.entries(properties).map(([key, field]) => <div key={key} style={{ marginBottom: 10 }}>{Object.hasOwn(obj, key) ? <><SettingField secret={secret} rootSchema={rootSchema} schema={field} value={obj[key]} label={field.title ?? key} onChange={(v) => onChange({ ...obj, [key]: v })} />{!schema.required?.includes(key) && <button className="ox-button" type="button" onClick={() => remove(key)}>Remove {field.title ?? key}</button>}</> : <button className="ox-button" type="button" onClick={() => onChange({ ...obj, [key]: emptyValue(field, rootSchema) })}>Add {field.title ?? key}</button>}</div>)}{Object.entries(obj).filter(([k]) => !Object.hasOwn(properties, k)).map(([key, v]) => <div key={key}><SettingField secret={secret} rootSchema={rootSchema} schema={typeof schema.additionalProperties === "object" ? schema.additionalProperties : { type: "string" }} value={v} label={key} onChange={(next) => onChange({ ...obj, [key]: next })} /><button className="ox-button" type="button" onClick={() => remove(key)}>Remove {key}</button></div>)}{schema.additionalProperties !== false && <div><label htmlFor={id}>New entry name</label><input id={id} className="ox-input" value={newKey} onChange={(e) => setNewKey(e.target.value)} /><button className="ox-button" type="button" disabled={!newKey.trim() || Object.hasOwn(obj, newKey)} onClick={() => { onChange(Object.assign(Object.create(null), obj, { [newKey]: emptyValue(typeof schema.additionalProperties === "object" ? schema.additionalProperties : { type: "string" }, rootSchema) })); setNewKey(""); }}>Add entry</button></div>}</fieldset>;
  }
  if (schema.contentMediaType === "text/markdown" && !secret) return <div><span>{label}</span><Markdown value={String(value ?? "")} onChange={onChange} /></div>;
  const numeric = type === "number" || type === "integer";
  return <label htmlFor={id} style={{ display: "grid", gap: 4 }}>{label}{multiline && !secret && !numeric ? <textarea id={id} className="ox-input" rows={5} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} /> : <input id={id} className="ox-input" type={secret ? "password" : numeric ? "number" : "text"} autoComplete={secret ? "new-password" : undefined} min={schema.minimum} max={schema.maximum} step={type === "integer" ? 1 : "any"} value={typeof value === "number" && !Number.isFinite(value) ? "" : String(value ?? "")} onChange={(e) => onChange(numeric ? e.target.value === "" ? undefined : Number(e.target.value) : e.target.value)} />}</label>;
}
