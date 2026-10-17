import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { AgentIcon } from "./AgentIcon.js";

afterEach(cleanup);

const MARK = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciLz4=";

// A harness declares its logo; without one the generic robot stands in.
test("without an icon the generic robot is drawn", () => {
  const { getByTestId, container } = render(<AgentIcon size={13} />);
  expect(getByTestId("agent-icon").getAttribute("data-icon")).toBe("fallback");
  expect(container.querySelector("svg")).not.toBeNull();
});

// The declared logo is only ever a picture: the data URI is the mask of a
// span, not markup, and it takes the text color.
test("a declared logo is drawn as an image, in the text color", () => {
  const { getByTestId, container } = render(<AgentIcon icon={MARK} size={13} />);
  const el = getByTestId("agent-icon");
  expect(el.getAttribute("data-icon")).toBe("declared");
  expect(el.getAttribute("style")).toContain(MARK);
  expect(el.getAttribute("style")?.toLowerCase()).toContain("currentcolor");
  expect(container.querySelector("svg")).toBeNull();
  expect(container.querySelector("script")).toBeNull();
});

// Only an SVG data URI is drawn; anything else (a URL, markup) falls back.
test("anything but an SVG data URI falls back to the robot", () => {
  for (const bad of ["https://evil.example/x.svg", "<svg onload=alert(1)/>", 'data:text/html,<script>x</script>', 'data:image/svg+xml;base64,AA");background:url(https://x']) {
    const { getByTestId, unmount } = render(<AgentIcon icon={bad} />);
    expect(getByTestId("agent-icon").getAttribute("data-icon")).toBe("fallback");
    unmount();
  }
});
