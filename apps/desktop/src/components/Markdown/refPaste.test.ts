import { expect, test } from "bun:test";

import { knowledgeUri } from "../../uri/uri.js";
import { pastedLink } from "./refPaste.js";

// What a paste into the editor is: an oxplow URI, a web URL, or neither.

test("a paste is a URI, a web URL, or plain text", () => {
  expect(pastedLink(` ${knowledgeUri("oxwiki", "notes")}\n`)).toEqual({ kind: "ref", uri: knowledgeUri("oxwiki", "notes") });
  expect(pastedLink("https://example.com/a")).toEqual({ kind: "url", url: "https://example.com/a" });
  expect(pastedLink("see https://example.com")).toBeNull();
  expect(pastedLink("oxplow:not a uri")).toBeNull();
});
