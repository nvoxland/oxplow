import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

const realApi = await import("./api.js");
mock.module("./api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    { name: "oxplow-foundation", enabled: true, pages: [{ id: "problems", icon: "triangle-alert" }] },
  ],
}));

const { PageKindIcon } = await import("./pageKinds.js");

afterEach(cleanup);

test("a tab for an extension's page draws the icon its page declares", async () => {
  const { container } = render(<PageKindIcon kind="ext-page" tabId="page:oxplow-foundation/problems" />);
  await waitFor(() => expect(container.querySelector("svg.lucide-triangle-alert")).not.toBeNull());
});

test("without a declared icon it draws the lens icon", () => {
  const { container } = render(<PageKindIcon kind="ext-page" tabId="page:oxplow-foundation/other" />);
  expect(container.querySelector("svg.lucide-glasses")).not.toBeNull();
});
