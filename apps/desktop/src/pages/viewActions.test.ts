import { expect, test } from "bun:test";
import { definitionUri, pageSlug, titledPage, viewWikiBody } from "./viewActions.js";
import { paramOf } from "../uri/uri.js";

test("a saved page keeps comments and ordered loaders when its title changes", () => {
  const yaml = titledPage("# Keep this\ntitle: Before\nloaders:\n  source: { rows: [] }\n  dependent: { sql: SELECT * FROM source }\nview: { viz: table, from: dependent }\n", "After");
  expect(yaml).toContain("# Keep this");
  expect(yaml).toContain("title: After");
  expect(yaml.indexOf("source:")).toBeLessThan(yaml.indexOf("dependent:"));
});

test("a wiki view's enclosing fence cannot be closed by markdown inside its YAML", () => {
  const source = "title: Example\nview: { markdown: '```example```' }\n";
  expect(viewWikiBody("Example", source)).toBe(`# Example\n\n\`\`\`\`oxplow-page\n${source}\`\`\`\`\n`);
  expect(pageSlug("Busy Tasks!")).toBe("busy-tasks");
  expect(pageSlug("2026 release")).toBe("page-2026-release");
});

test("Show definition carries exact params and inline source, or the deployed page", () => {
  const uri = definitionUri("inline", { q: "two words" }, "title: Inline");
  expect(paramOf(uri, "spec")).toBe("title: Inline");
  expect(paramOf(uri, "params")).toBe('{"q":"two words"}');
  expect(paramOf(definitionUri("acme/list", {}), "of")).toBe("acme/list");
});
