import type { Schema } from "../settingsModel.js";

/** What every setting control receives. `schema` is the schema as declared (not yet resolved). */
export type FieldProps = {
  schema: Schema;
  value: unknown;
  onChange(value: unknown): void;
  label: string;
  secret?: boolean;
  multiline?: boolean;
  rootSchema?: Schema;
};

/** What the dispatcher works out once, so each control does not repeat it. */
export type FieldInfo = {
  /** The schema with `$ref` resolved and a single non-null branch merged in. */
  schema: Schema;
  /** The schema with `$ref` resolved only. */
  resolved: Schema;
  /** Alternatives (oneOf, anyOf, type lists) without the null branch. */
  branches: Schema[];
  /** The first non-null JSON type of `schema`. */
  type: string | undefined;
  rootSchema: Schema;
};

export type ControlProps = Omit<FieldProps, "schema" | "rootSchema"> & { info: FieldInfo };
