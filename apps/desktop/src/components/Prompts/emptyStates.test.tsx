import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { Stream, Thread } from "../../api.js";

// `.context/usability.md` → "Empty states": every empty page or section is
// an `EmptyState` (one mechanism for empty copy), which marks its root
// with `data-empty-state`. These mount the surfaces that can be mounted
// cheaply and look for that mark where the old plain copy was.

const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({
  ...realApi,
  subscribeOxplowEvents: () => () => {},
  listRecentPageVisits: async () => [],
  topVisitedPages: async () => [],
  subscribePageVisitEvents: () => () => {},
}));

const { BacklinksList } = await import("../../tabs/BacklinksList.js");

afterEach(cleanup);

const stream = { id: "str1", name: "main", kind: "primary" } as unknown as Stream;
const emptyStates = (container: HTMLElement) => Array.from(container.querySelectorAll("[data-empty-state]"));

test("an empty backlinks list is an EmptyState", () => {
  const { container } = render(<BacklinksList entries={[]} onOpenPage={() => {}} />);
  const empty = container.querySelector('[data-testid="backlinks-list-empty"]');
  expect(empty?.hasAttribute("data-empty-state")).toBe(true);
});
