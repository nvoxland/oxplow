import { describe, expect, test } from "bun:test";
import { draftsFrom, fieldsOf, valueOf } from "./schemaFormModel.js";

/** What schemars writes for a command input: `$defs`, `Option<T>`, an enum. */
const schema = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  type: "object",
  required: ["title", "to"],
  properties: {
    title: { type: "string", description: "What it's called." },
    body: { type: ["string", "null"] },
    to: { $ref: "#/$defs/State" },
    priority: { anyOf: [{ $ref: "#/$defs/Priority" }, { type: "null" }] },
    count: { type: "integer" },
    labels: { type: "array", items: { type: "string" } },
    urgent: { type: "boolean" },
    meta: { type: "object", additionalProperties: true },
    target: { type: "object", required: ["team"], properties: { team: { type: "string" } } },
  },
  $defs: {
    State: { type: "string", enum: ["todo", "done"] },
    Priority: { type: "string", enum: ["low", "high"] },
  },
};

describe("fieldsOf", () => {
  test("reads the schemars shapes", () => {
    const f = fieldsOf(schema);
    const by = (p: string) => f.find((x) => x.path === p)!;
    expect(by("title")).toMatchObject({ kind: "text", required: true, label: "Title", description: "What it's called." });
    expect(by("body")).toMatchObject({ kind: "text", required: false });
    expect(by("to")).toMatchObject({ kind: "enum", required: true, options: ["todo", "done"] });
    expect(by("priority")).toMatchObject({ kind: "enum", required: false, options: ["low", "high"] });
    expect(by("count").kind).toBe("integer");
    expect(by("labels").kind).toBe("strings");
    expect(by("urgent").kind).toBe("boolean");
    expect(by("meta").kind).toBe("json");
    expect(by("target").children.map((c) => [c.path, c.required])).toEqual([["target.team", true]]);
  });
});

describe("valueOf", () => {
  const fields = fieldsOf(schema);
  test("builds the value, leaving empty optional fields out", () => {
    const { value, errors } = valueOf(fields, {
      title: "Fix it",
      to: "done",
      count: "3",
      labels: "a\n\n b ",
      urgent: "true",
      "target.team": "core",
    });
    expect(errors).toEqual({});
    expect(value).toEqual({ title: "Fix it", to: "done", count: 3, labels: ["a", "b"], urgent: true, target: { team: "core" } });
  });

  test("names each field's problem", () => {
    const { errors } = valueOf(fields, { to: "later", count: "1.5", meta: "{nope" });
    expect(errors).toEqual({
      title: "Title is required",
      to: "To must be one of todo, done",
      count: "Count must be a whole number",
      meta: "Meta isn't valid JSON",
      "target.team": "Team is required",
    });
  });

  test("drafts round-trip an existing value", () => {
    const existing = { title: "x", to: "todo", labels: ["a", "b"], meta: { k: 1 }, target: { team: "t" }, urgent: false };
    const { value, errors } = valueOf(fields, draftsFrom(fields, existing));
    expect(errors).toEqual({});
    expect(value).toEqual(existing);
  });
});
