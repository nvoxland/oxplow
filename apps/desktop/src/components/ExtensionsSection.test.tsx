import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import type { Extension, ExtensionReview } from "../api.js";
import { ReviewPanel } from "./ExtensionsSection.js";

afterEach(cleanup);

const review = (over: Partial<Extension> = {}): ExtensionReview => ({
  extension: {
    name: "shared",
    description: "Shared lenses",
    path: "oxplow/extensions/shared",
    errors: [],
    lenses: [],
    source: null,
    sources: [],
    origin: "project",
    ui: { slots: [], commands: [], decorators: [] },
    enabled: true,
    advisories: [],
    measures: [],
    metrics: [],
    gauges: [],
    dimensions: [],
    ...over,
  } as Extension,
  git: "https://github.com/acme/lenses",
  gitRef: null,
  sha: "0123456789abcdef0123456789abcdef01234567",
  problems: [],
});

// tsk378: an install shows what it brings in; the person confirms or cancels.
test("the review panel confirms, cancels on Escape, and blocks on load errors", () => {
  let confirmed = 0;
  let cancelled = 0;
  const { getByTestId, rerender } = render(
    <ReviewPanel review={review()} action="Install" busy={false} onConfirm={() => confirmed++} onCancel={() => cancelled++} />,
  );
  expect(getByTestId("extension-review").textContent).toContain("shared");
  const confirm = getByTestId("extension-review-confirm") as HTMLButtonElement;
  expect(document.activeElement).toBe(confirm);
  fireEvent.click(confirm);
  expect(confirmed).toBe(1);
  fireEvent.keyDown(confirm, { key: "Escape" });
  expect(cancelled).toBe(1);

  rerender(
    <ReviewPanel
      review={review({ errors: ["extension.yaml: unknown field `bogus`"] })}
      action="Install"
      busy={false}
      onConfirm={() => confirmed++}
      onCancel={() => cancelled++}
    />,
  );
  expect((getByTestId("extension-review-confirm") as HTMLButtonElement).disabled).toBe(true);
  expect(getByTestId("extension-review").textContent).toContain("bogus");
});
