import { afterEach, expect, test } from "bun:test";

import { isStoreFile, recordLabel, recordLinkText, recordOf, recordOfAnyBareId, recordOfBareId, recordOfStoreId, recordStores, setRecordStores, type RecordStore } from "./recordStores.js";
import { resourceUri } from "./uri/uri.js";

// What a store's declared ids name, for any model: no store, resource or id
// is built in.

const before = recordStores();
afterEach(() => setRecordStores(before));

const store = (over: Partial<RecordStore>): RecordStore => ({
  model: "acme/ticket_record",
  store: "core",
  resource: "acme/ticket",
  storeParam: "team",
  key: "id",
  idPattern: "AC-[0-9]+",
  loose: true,
  files: null,
  ...over,
});

test("a bare id names the first store whose pattern matches it whole, the loose ones first", () => {
  setRecordStores([
    store({ store: "wiki", model: "acme/page_record", resource: "acme/page", storeParam: "store", idPattern: "[a-z0-9-]+", loose: false }),
    store({ store: "core" }),
    store({ store: "ops", idPattern: "OP-[0-9]+" }),
  ]);
  expect(recordOfBareId("AC-7", true)).toBe(resourceUri("acme/ticket", { id: "AC-7", team: "core" }));
  expect(recordOfBareId("OP-1", true)).toBe(resourceUri("acme/ticket", { id: "OP-1", team: "ops" }));
  expect(recordOfBareId("AC-7x", true)).toBeNull();
  expect(recordOfBareId("some-page", true)).toBeNull();
  expect(recordOfBareId("some-page", false)).toBe(resourceUri("acme/page", { id: "some-page", store: "wiki" }));
  expect(recordOfAnyBareId("ac-7")).toBe(resourceUri("acme/page", { id: "ac-7", store: "wiki" }));
});

test("a store with no pattern, or one that isn't a regex, names no bare id", () => {
  setRecordStores([store({ idPattern: null }), store({ store: "bad", idPattern: "(" })]);
  expect(recordOfAnyBareId("AC-7")).toBeNull();
  expect(recordOfStoreId("core", "AC-7")).toBe(resourceUri("acme/ticket", { id: "AC-7", team: "core" }));
  expect(recordOfStoreId("nope", "AC-7")).toBeNull();
});

test("a URI is a record when its store is one, and is written as its id or store:id", () => {
  setRecordStores([store({}), store({ store: "odd", idPattern: null })]);
  const uri = resourceUri("acme/ticket", { id: "AC-7", team: "core" });
  expect(recordOf(uri)).toEqual({ store: "core", id: "AC-7", resource: "acme/ticket" });
  expect(recordOf(resourceUri("acme/ticket", { id: "AC-7", team: "gone" }))).toBeNull();
  expect(recordLabel(uri)).toBe("AC-7");
  expect(recordLinkText(uri)).toBe("AC-7");
  expect(recordLinkText(resourceUri("acme/ticket", { id: "x", team: "odd" }))).toBe("odd:x");
});

test("a file is a store's when it is <folder>/<id><suffix> of a store that keeps rows as files", () => {
  setRecordStores([store({ store: "wiki", files: { folder: ".oxplow/wiki", suffix: ".md" } }), store({ store: "plain" })]);
  expect(isStoreFile(".oxplow/wiki/kiwi.md")).toBe(true);
  expect(isStoreFile(".oxplow/wiki/sub/kiwi.md")).toBe(false);
  expect(isStoreFile(".oxplow/wiki/kiwi.txt")).toBe(false);
  expect(isStoreFile(".oxplow/other/kiwi.md")).toBe(false);
  expect(isStoreFile("kiwi.md")).toBe(false);
});
