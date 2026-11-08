import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

const realApi = await import("./api.js");
const pristineApi = { ...realApi };
mock.module("./api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    { name: "oxplow-foundation", namespace: "oxplow", enabled: true, pageFiles: [{ id: "oxplow/problems", extension: "oxplow-foundation", slug: "problems", title: "Problems", icon: "triangle-alert" }] },
  ],
}));

const { PageKindIcon } = await import("./pageKinds.js");

afterEach(cleanup);

test("a declared page draws the icon its declaration names", async () => {
  const { container } = render(<PageKindIcon kind="problems" />);
  await waitFor(() => expect(container.querySelector("svg.lucide-triangle-alert")).not.toBeNull());
});

test("a page no extension declares draws the page icon", () => {
  const { container } = render(<PageKindIcon kind="page" />);
  expect(container.querySelector("svg.lucide-glasses")).not.toBeNull();
});

// `mock.module` is process-wide: put the real module back so the files that
// run after this one don't see this file's extensions.
afterAll(() => {
  mock.module("./api.js", () => pristineApi);
});
