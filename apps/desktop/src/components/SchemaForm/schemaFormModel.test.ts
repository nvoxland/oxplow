import { describe, expect, test } from "bun:test";
import { draftsFrom, fieldsOf, hiddenOf, sameDrafts, shownFields, valueOf, withSubfields } from "./schemaFormModel.js";

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

describe("shown fields", () => {
  const shown = shownFields(["kind", { name: "url", visible_if: { kind: ["http", "https"] } }, { name: "token", secret: true, visible_if: { url: "x" } }]);
  test("a name or a spec, conditions as lists of text", () => {
    expect(shown.map((f) => [f.name, f.secret, f.visibleIf])).toEqual([
      ["kind", false, {}],
      ["url", false, { kind: ["http", "https"] }],
      ["token", true, { url: ["x"] }],
    ]);
  });
  test("a field is hidden by its condition, or by its condition's field being hidden", () => {
    expect([...hiddenOf(shown, { kind: "http", url: "x" })]).toEqual([]);
    expect([...hiddenOf(shown, { kind: "file", url: "x" })]).toEqual(["url", "token"]);
  });
  test("drafts are the same when they hold the same values", () => {
    expect(sameDrafts({ a: "1", b: "" }, { a: "1" })).toBe(true);
    expect(sameDrafts({ a: "1" }, { a: "2" })).toBe(false);
  });
});

describe("withSubfields", () => {
  const sub = [
    { name: "priority", title: "Priority", kind: "enum", values: ["high", "low"], read_only: false },
    { name: "points", title: "Story points", kind: "number", values: [], read_only: false },
    { name: "label", title: "Label", kind: "text", values: [], read_only: false },
    { name: "reporter", title: "Reporter", kind: "text", values: [], read_only: true },
  ];

  test("an open object field gets the rows as its children, read-only ones left out", () => {
    const fields = fieldsOf(withSubfields(schema, { meta: sub }));
    const meta = fields.find((f) => f.path === "meta")!;
    expect(meta.kind).toBe("object");
    expect(meta.children.map((c) => [c.path, c.kind, c.label, c.options])).toEqual([
      ["meta.priority", "enum", "Priority", ["high", "low"]],
      ["meta.points", "number", "Story points", []],
      ["meta.label", "text", "Label", []],
    ]);
  });

  test("its value is the object of what was filled in, and nothing when empty", () => {
    const fields = fieldsOf(withSubfields(schema, { meta: sub })).filter((f) => f.path === "meta");
    expect(valueOf(fields, {}).value).toEqual({});
    expect(valueOf(fields, { "meta.priority": "high", "meta.points": "3" }).value).toEqual({
      meta: { priority: "high", points: 3 },
    });
    expect(draftsFrom(fields, { meta: { priority: "low" } })).toEqual({ "meta.priority": "low" });
  });

  test("a field the schema doesn't have is left alone", () => {
    expect(withSubfields(schema, { nope: sub })).toEqual(schema);
  });
});
