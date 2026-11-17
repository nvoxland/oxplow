import type { VizRun } from "../api.js";
import type { Level } from "../tauri-bridge/generated/bindings.js";

export type { Level };
export const LEVELS: Level[] = ["local", "project", "global"];
export type Schema = { $ref?: string; $defs?: Record<string, Schema>; definitions?: Record<string, Schema>; uniqueItems?: boolean; type?: string | string[]; const?: unknown; title?: string; description?: string; default?: unknown; enum?: unknown[]; properties?: Record<string, Schema>; required?: string[]; additionalProperties?: boolean | Schema; items?: Schema; minimum?: number; maximum?: number; minItems?: number; maxItems?: number; minLength?: number; contentMediaType?: string; oneOf?: Schema[]; anyOf?: Schema[] };
export interface Setting {
  key: string; title: string; doc: string; category: string; schema: Schema; ui: Record<string, unknown>;
  value: unknown; origin: string | null; owner: string; isSet: boolean; secret: boolean; unavailable: boolean; defaultValue: unknown;
  overrides: Partial<Record<Level, unknown>>; statuses: Record<Level, string>; restartRequired: boolean;
}
const decode = (v: unknown): unknown => { if (typeof v !== "string") return v; try { return JSON.parse(v); } catch { return v; } };
export function inventory(run: VizRun): Setting[] {
  return run.result.rows.map((row) => {
    const r = Object.fromEntries(run.result.columns.map((c, i) => [c, row[i]]));
    const schema = (decode(r.schema) ?? {}) as Schema;
    const overrides: Setting["overrides"] = {};
    const statuses = {} as Setting["statuses"];
    for (const level of LEVELS) {
      const raw = r[`${level}_value`];
      if (raw !== null && raw !== undefined) overrides[level] = decode(raw);
      statuses[level] = String(r[`${level}_status`] ?? (Object.hasOwn(overrides, level) ? "set" : "unset"));
    }
    return { key: String(r.key), title: String(r.title || r.key), doc: String(r.doc ?? r.description ?? ""), category: String(r.category ?? ""), schema,
      ui: (decode(r.ui) ?? {}) as Record<string, unknown>, value: r.value == null && r.is_set !== 1 && r.is_set !== true ? undefined : decode(r.value), origin: r.origin == null ? null : String(r.origin), secret: r.secret === 1 || r.secret === true,
      owner: String(r.owner ?? r.extension ?? "core"), isSet: r.is_set == null ? r.origin != null && r.origin !== "default" : r.is_set === 1 || r.is_set === true,
      unavailable: r.unavailable === 1 || r.unavailable === true, defaultValue: r.default_value == null ? schema.default : decode(r.default_value), overrides, statuses,
      restartRequired: r.restart_required === 1 || r.restart_required === true };
  });
}
export const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
/** Resolve a local JSON Schema pointer against the setting's complete registry schema. */
export function resolveSchema(schema: Schema, root: Schema = schema): Schema {
  const visited = new Set<string>();
  let next = schema;
  while (next.$ref?.startsWith("#/") && !visited.has(next.$ref)) {
    visited.add(next.$ref);
    let target: unknown = root;
    for (const token of next.$ref.slice(2).split("/")) target = target && typeof target === "object" ? (target as Record<string, unknown>)[token.replace(/~1/g, "/").replace(/~0/g, "~")] : undefined;
    if (!target || typeof target !== "object") break;
    const { $ref: ignored, ...siblings } = next;
    void ignored;
    next = { ...(target as Schema), ...siblings };
  }
  return next;
}
export function alternatives(schema: Schema, root: Schema = schema): Schema[] {
  schema = resolveSchema(schema, root);
  return (schema.anyOf ?? schema.oneOf ?? (Array.isArray(schema.type) ? schema.type.map((type) => ({ ...schema, type })) : [])).map((s) => resolveSchema(s, root));
}
export function concreteSchema(schema: Schema, root: Schema = schema): Schema {
  schema = resolveSchema(schema, root);
  const branches = alternatives(schema, root).filter((s) => s.type !== "null");
  return branches.length === 1 ? { ...schema, ...branches[0], anyOf: undefined, oneOf: undefined } : schema;
}
export function emptyValue(schema: Schema, root: Schema = schema): unknown {
  schema = concreteSchema(schema, root);
  const branches = alternatives(schema, root);
  if (branches.length) return emptyValue(branches.find((s) => s.type !== "null") ?? branches[0]!, root);
  if (schema.const !== undefined) return schema.const;
  if (schema.default !== undefined) return schema.default;
  if (schema.enum?.length) return schema.enum[0];
  const type = Array.isArray(schema.type) ? schema.type.find((t) => t !== "null") : schema.type;
  if (type === "object" || schema.properties) return Object.fromEntries(Object.entries(schema.properties ?? {}).filter(([k]) => schema.required?.includes(k)).map(([k, s]) => [k, emptyValue(s, root)]));
  if (type === "array") return [];
  if (type === "boolean") return false;
  if (type === "number" || type === "integer") return schema.minimum ?? 0;
  return "";
}
export function validate(schema: Schema, value: unknown, path = "Value", root: Schema = schema): string[] {
  schema = resolveSchema(schema, root);
  const branches = alternatives(schema, root);
  if (branches.length) {
    if (branches.some((s) => !validate(s, value, path, root).length)) return [];
    return [`${path}: choose a supported value and complete its fields.`];
  }
  if (value === null && (schema.type === "null" || (Array.isArray(schema.type) && schema.type.includes("null")) || (schema.anyOf ?? schema.oneOf)?.some((s) => s.type === "null"))) return [];
  schema = concreteSchema(schema, root);
  const errors: string[] = [];
  if (schema.const !== undefined && !same(schema.const, value)) errors.push(`${path}: this value is fixed.`);
  if (schema.enum && !schema.enum.some((v) => same(v, value))) errors.push(`${path}: choose a listed value.`);
  const type = Array.isArray(schema.type) ? schema.type.find((t) => t !== "null") : schema.type;
  const actual = value === null ? "null" : Array.isArray(value) ? "array" : typeof value;
  if (type && (type === "integer" ? !Number.isInteger(value) : actual !== type)) return [`${path}: enter a ${type}.`];
  if (typeof value === "number" && (!Number.isFinite(value) || (schema.minimum !== undefined && value < schema.minimum) || (schema.maximum !== undefined && value > schema.maximum))) errors.push(`${path}: enter a number${schema.minimum !== undefined ? ` at least ${schema.minimum}` : ""}${schema.maximum !== undefined ? ` at most ${schema.maximum}` : ""}.`);
  if (typeof value === "string" && schema.minLength !== undefined && value.length < schema.minLength) errors.push(`${path}: enter at least ${schema.minLength} characters.`);
  if (Array.isArray(value)) {
    if (schema.minItems !== undefined && value.length < schema.minItems) errors.push(`${path}: add at least ${schema.minItems} items.`);
    if (schema.maxItems !== undefined && value.length > schema.maxItems) errors.push(`${path}: keep at most ${schema.maxItems} items.`);
    if (schema.uniqueItems && new Set(value.map((v) => JSON.stringify(v))).size !== value.length) errors.push(`${path}: each item must be unique.`);
    value.forEach((v, i) => errors.push(...validate(schema.items ?? {}, v, `${path} ${i + 1}`, root)));
  } else if (value && typeof value === "object") {
    const obj = value as Record<string, unknown>;
    for (const key of schema.required ?? []) if (!Object.hasOwn(obj, key)) errors.push(`${path}: ${key} is required.`);
    if (schema.additionalProperties === false) for (const key of Object.keys(obj)) if (!Object.hasOwn(schema.properties ?? {}, key)) errors.push(`${path}: ${key} is not an allowed field.`);
    for (const [key, v] of Object.entries(obj)) errors.push(...validate(schema.properties?.[key] ?? (typeof schema.additionalProperties === "object" ? schema.additionalProperties : {}), v, `${path}.${key}`, root));
  }
  return errors;
}
export function displayValue(value: unknown): string {
  if (value === undefined) return "Not set";
  if (value === null) return "Null";
  if (value === "") return "Empty string";
  if (Array.isArray(value)) return value.length ? value.map(displayValue).join(" · ") : "Empty list";
  if (typeof value === "object") return Object.entries(value).length ? Object.entries(value).map(([k, v]) => `${k}: ${displayValue(v)}`).join(" · ") : "Empty object";
  return String(value);
}
