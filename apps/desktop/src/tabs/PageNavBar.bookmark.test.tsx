import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageNavBar } from "./PageNavBar.js";

afterEach(cleanup);

const bar = (bookmark: React.ComponentProps<typeof PageNavBar>["bookmark"]) =>
  render(<PageNavBar canBack={false} canForward={false} onBack={() => {}} onForward={() => {}} bookmark={bookmark} />);

// A bookmark is of a page at its view; the same page starred at another
// view says so and offers to move that star here.
test("a page bookmarked at other values says so and updates that bookmark", () => {
  let updated = 0;
  const view = bar({ scope: null, onToggleScope: () => {}, other: { scope: "stream" }, onUpdateOther: () => updated++ });
  const star = view.getByTestId("page-nav-bookmark");
  expect(star.textContent).toBe("☆");
  expect(star.getAttribute("title")).toBe("Bookmarked at other values");
  fireEvent.click(star);
  fireEvent.click(view.getByTestId("page-nav-bookmark-update"));
  expect(updated).toBe(1);
});

test("a page not bookmarked anywhere offers no update", () => {
  const view = bar({ scope: null, onToggleScope: () => {}, other: null, onUpdateOther: () => {} });
  expect(view.getByTestId("page-nav-bookmark").getAttribute("title")).toBe("Bookmark");
  fireEvent.click(view.getByTestId("page-nav-bookmark"));
  expect(view.queryByTestId("page-nav-bookmark-update")).toBeNull();
});
