/// A JSON Schema as form fields (P6.B2): what `SchemaForm` renders for a
/// command's input or a provider's config. Pure, so the shaping, the
/// values and the errors are tested without a DOM.
///
/// It reads what schemars writes (draft 2020-12): `properties`, `required`,
/// `$defs` + `$ref`, `Option<T>` as `type: [T, "null"]` or `anyOf` with a
/// null branch, enums as `enum`, and `description` / `default`. A shape it
/// doesn't know becomes a JSON field, never a silent drop.

export type Schema = Record<string, unknown>;

export type FieldKind = "text" | "markdown" | "integer" | "number" | "boolean" | "enum" | "strings" | "object" | "json";

export interface Field {
  /** Dotted path from the root object (`a.b`). */
  path: string;
  key: string;
  label: string;
  description: string | null;
  kind: FieldKind;
  required: boolean;
  /** `enum`: the choices. */
  options: string[];
  /** `object`: its fields. */
  children: Field[];
}

/** Draft text per field path — what the inputs hold. */
export type Drafts = Record<string, string>;

function resolve(schema: Schema, root: Schema): Schema {
  const ref = schema.$ref;
  if (typeof ref === "string" && ref.startsWith("#/$defs/")) {
    const defs = (root.$defs ?? {}) as Record<string, Schema>;
    const target = defs[ref.slice("#/$defs/".length)];
    if (target) return resolve({ ...target, ...withoutRef(schema) }, root);
  }
  // `Option<T>`: `anyOf: [T, {type: null}]` (or `oneOf`).
  for (const key of ["anyOf", "oneOf"] as const) {
    const branches = schema[key];
    if (Array.isArray(branches)) {
      const nonNull = (branches as Schema[]).filter((b) => b.type !== "null");
      if (nonNull.length === 1 && nonNull.length < branches.length) {
        return resolve({ ...withoutKey(schema, key), ...nonNull[0], nullable: true }, root);
      }
    }
  }
  return schema;
}

function withoutRef(s: Schema): Schema {
  const { $ref: _ref, ...rest } = s;
  return rest;
}

function withoutKey(s: Schema, k: string): Schema {
  const out = { ...s };
  delete out[k];
  return out;
}

/** The schema's `type`, ignoring `null` (an optional value). */
function typeOf(s: Schema): string | null {
  const t = s.type;
  if (typeof t === "string") return t;
  if (Array.isArray(t)) return (t.find((x) => x !== "null") as string | undefined) ?? null;
  return null;
}

function kindOf(s: Schema, root: Schema): FieldKind {
  if (Array.isArray(s.enum) && s.enum.every((v) => typeof v === "string")) return "enum";
  switch (typeOf(s)) {
    case "string":
      return s.contentMediaType === "text/markdown" ? "markdown" : "text";
    case "integer":
      return "integer";
    case "number":
      return "number";
    case "boolean":
      return "boolean";
    case "array": {
      const items = s.items ? resolve(s.items as Schema, root) : null;
      return items && typeOf(items) === "string" && !Array.isArray(items.enum) ? "strings" : "json";
    }
    case "object":
      return s.properties ? "object" : "json";
    default:
      return "json";
  }
}

function labelOf(key: string): string {
  const words = key.replace(/([a-z0-9])([A-Z])/g, "$1 $2").replace(/[_-]+/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The root object's fields (a non-object root is one JSON field). */
export function fieldsOf(schema: Schema): Field[] {
  return fieldsIn(resolve(schema, schema), schema, "");
}

function fieldsIn(s: Schema, root: Schema, prefix: string): Field[] {
  const props = (s.properties ?? {}) as Record<string, Schema>;
  const required = new Set((s.required ?? []) as string[]);
  return Object.entries(props).map(([key, raw]) => {
    const prop = resolve(raw, root);
    const path = prefix ? `${prefix}.${key}` : key;
    const kind = kindOf(prop, root);
    return {
      path,
      key,
      label: typeof prop.title === "string" ? prop.title : labelOf(key),
      description: typeof prop.description === "string" ? prop.description : null,
      kind,
      required: required.has(key) && prop.nullable !== true && !(Array.isArray(prop.type) && prop.type.includes("null")),
      options: kind === "enum" ? (prop.enum as string[]) : [],
      children: kind === "object" ? fieldsIn(prop, root, path) : [],
    };
  });
}

function getAt(value: unknown, path: string): unknown {
  let cur: unknown = value;
  for (const k of path.split(".")) {
    if (cur === null || typeof cur !== "object") return undefined;
    cur = (cur as Record<string, unknown>)[k];
  }
  return cur;
}

/** Drafts for `fields` from an existing value (a config, a form's
 *  defaults). */
export function draftsFrom(fields: Field[], value: unknown): Drafts {
  const out: Drafts = {};
  const walk = (fs: Field[]) => {
    for (const f of fs) {
      if (f.kind === "object") {
        walk(f.children);
        continue;
      }
      const v = getAt(value, f.path);
      if (v === undefined || v === null) continue;
      switch (f.kind) {
        case "strings":
          out[f.path] = Array.isArray(v) ? v.map(String).join("\n") : String(v);
          break;
        case "json":
          out[f.path] = JSON.stringify(v, null, 2);
          break;
        default:
          out[f.path] = String(v);
      }
    }
  };
  walk(fields);
  return out;
}

/** The value `drafts` make, and each field's error (none when valid). An
 *  empty optional field is left out. */
export function valueOf(fields: Field[], drafts: Drafts): { value: Record<string, unknown>; errors: Record<string, string> } {
  const errors: Record<string, string> = {};
  const build = (fs: Field[]): Record<string, unknown> => {
    const out: Record<string, unknown> = {};
    for (const f of fs) {
      if (f.kind === "object") {
        const inner = build(f.children);
        if (Object.keys(inner).length > 0 || f.required) out[f.key] = inner;
        continue;
      }
      const raw = drafts[f.path] ?? "";
      if (f.kind === "boolean") {
        if (raw !== "" || f.required) out[f.key] = raw === "true";
        continue;
      }
      if (raw.trim() === "") {
        if (f.required) errors[f.path] = `${f.label} is required`;
        continue;
      }
      switch (f.kind) {
        case "integer": {
          const n = Number(raw);
          if (!Number.isInteger(n)) errors[f.path] = `${f.label} must be a whole number`;
          else out[f.key] = n;
          break;
        }
        case "number": {
          const n = Number(raw);
          if (Number.isNaN(n)) errors[f.path] = `${f.label} must be a number`;
          else out[f.key] = n;
          break;
        }
        case "strings":
          out[f.key] = raw
            .split("\n")
            .map((l) => l.trim())
            .filter((l) => l !== "");
          break;
        case "json":
          try {
            out[f.key] = JSON.parse(raw);
          } catch {
            errors[f.path] = `${f.label} isn't valid JSON`;
          }
          break;
        case "enum":
          if (!f.options.includes(raw)) errors[f.path] = `${f.label} must be one of ${f.options.join(", ")}`;
          else out[f.key] = raw;
          break;
        default:
          out[f.key] = raw;
      }
    }
    return out;
  };
  const value = build(fields);
  return { value, errors };
}

/** A value a `visible_if` condition compares a field's with. */
export type Scalar = string | number | boolean;

/** One of the fields a form shows: its name, or its name and how it
 *  shows (a lens form's `fields` entry). */
export type FieldSpec = string | { name: string; visible_if?: Record<string, Scalar | Scalar[]>; secret?: boolean };

export interface ShownField {
  name: string;
  /** Shown only while each named field holds one of these. */
  visibleIf: Record<string, string[]>;
  /** Masked, never in a draft; blank leaves it out. */
  secret: boolean;
}

export function shownFields(specs: FieldSpec[]): ShownField[] {
  return specs.map((s) =>
    typeof s === "string"
      ? { name: s, visibleIf: {}, secret: false }
      : {
          name: s.name,
          visibleIf: Object.fromEntries(
            Object.entries(s.visible_if ?? {}).map(([k, v]) => [k, (Array.isArray(v) ? v : [v]).map(String)]),
          ),
          secret: s.secret === true,
        },
  );
}

/** The shown fields hidden by their conditions over `drafts`: one whose
 *  condition's field is itself hidden is hidden too. */
export function hiddenOf(shown: ShownField[], drafts: Drafts): Set<string> {
  const hidden = new Set<string>();
  const byName = new Map(shown.map((f) => [f.name, f]));
  const isHidden = (f: ShownField, seen: Set<string>): boolean =>
    Object.entries(f.visibleIf).some(([on, values]) => {
      const other = byName.get(on);
      if (other && !seen.has(on) && isHidden(other, new Set(seen).add(on))) return true;
      return !values.includes(drafts[on] ?? "");
    });
  for (const f of shown) if (isHidden(f, new Set([f.name]))) hidden.add(f.name);
  return hidden;
}

/** Whether two drafts hold the same values (an emptied field is no value). */
export function sameDrafts(a: Drafts, b: Drafts): boolean {
  const filled = (d: Drafts) => Object.entries(d).filter(([, v]) => v !== "").sort(([x], [y]) => x.localeCompare(y));
  return JSON.stringify(filled(a)) === JSON.stringify(filled(b));
}
