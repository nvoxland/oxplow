import {
  AlternativesField,
  ArrayField,
  BooleanField,
  EnumField,
  NullableField,
  ObjectField,
  TextField,
  type FieldInfo,
  type FieldProps,
} from "./settingFields/index.js";
import { alternatives, choices, concreteSchema, resolveSchema } from "./settingsModel.js";

/** Structured controls retain JSON types; collections are edited as one value. */
export function SettingField({ schema: original, rootSchema = original, ...rest }: FieldProps) {
  const schema = concreteSchema(original, rootSchema);
  if ("const" in schema) {
    return (
      <p>
        {rest.label}: {String((schema as { const: unknown }).const)} (fixed)
      </p>
    );
  }
  const resolved = resolveSchema(original, rootSchema);
  const all = alternatives(resolved, rootSchema);
  const branches = all.filter((s) => s.type !== "null");
  const nullable = (Array.isArray(resolved.type) && resolved.type.includes("null")) || all.some((s) => s.type === "null");
  const type = Array.isArray(schema.type) ? schema.type.find((t) => t !== "null") : schema.type;
  const info: FieldInfo = { schema, resolved, branches, type, rootSchema };
  const props = { ...rest, info };
  if (nullable) return <NullableField {...props} />;
  const options = choices(original, rootSchema);
  if (options) return <EnumField {...props} choices={options} />;
  if (branches.length > 1) return <AlternativesField {...props} />;
  if (type === "boolean") return <BooleanField {...props} />;
  if (type === "array") return <ArrayField {...props} />;
  if (type === "object" || schema.properties) return <ObjectField {...props} />;
  return <TextField {...props} />;
}
