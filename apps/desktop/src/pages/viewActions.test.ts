import { expect, test } from "bun:test";
import { definitionUri, pageSlug, titledPage, viewWikiBody } from "./viewActions.js";
import { identityOf, viewOf } from "../uri/uri.js";

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

test("Definition in About is the running copy's file drawn as Definition with the run's exact params", () => {
  const running = "oxplow:oxplow/file?path=acme%2Fpages%2Flist.yaml&rev=abc123&filesystem=oxplow:extensions";
  const uri = definitionUri(running, { q: "two words" });
  expect(identityOf(uri)).toBe(identityOf(running));
  expect(viewOf(uri, "oxplow.renderer")).toBe("oxplow/definition");
  expect(viewOf(uri, "params")).toBe('{"q":"two words"}');
});
