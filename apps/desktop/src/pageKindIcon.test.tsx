import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

const realApi = await import("./api.js");
mock.module("./api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    { name: "oxplow-foundation", enabled: true, pages: [{ id: "problems", pageRef: "page:problems", title: "Problems", icon: "triangle-alert" }] },
  ],
}));

const { PageKindIcon } = await import("./pageKinds.js");

afterEach(cleanup);

test("a declared page draws the icon its declaration names", async () => {
  const { container } = render(<PageKindIcon kind="problems" />);
  await waitFor(() => expect(container.querySelector("svg.lucide-triangle-alert")).not.toBeNull());
});

test("a page no extension declares draws the lens icon", () => {
  const { container } = render(<PageKindIcon kind="page" />);
  expect(container.querySelector("svg.lucide-glasses")).not.toBeNull();
});
