import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";

// A markdown file drawn with its URI as `base`: a relative image is read
// from the file's filesystem and revision and drawn from its bytes, and a
// relative link opens that file. A web image stays a plain <img>.

const reads: { path: string; at: { rev?: string; filesystem?: string } }[] = [];
const realApi = await import("../../api.js");
const pristineApi = { ...realApi };
mock.module("../../api.js", () => ({
  ...realApi,
  readBytesAt: async (_stream: string, path: string, at: { rev?: string; filesystem?: string }) => {
    reads.push({ path, at });
    return path === "docs/img/gone.png" ? null : new Uint8Array([0x89, 0x50]);
  },
}));
const { Markdown } = await import("./Markdown.js");

const realCreate = URL.createObjectURL;
const realRevoke = URL.revokeObjectURL;
URL.createObjectURL = (blob: Blob) => `blob:test-${blob.type}`;
URL.revokeObjectURL = () => {};
afterAll(() => {
  mock.module("../../api.js", () => pristineApi);
  URL.createObjectURL = realCreate;
  URL.revokeObjectURL = realRevoke;
});
afterEach(() => {
  cleanup();
  reads.length = 0;
});

const settle = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });

test("a relative image is drawn from the base file's folder, and a relative link opens that file", async () => {
  const opened: string[] = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string) => opened.push(ref),
  } as unknown as PageNavigation;
  const base = "oxplow:oxplow/file?path=docs/README.md&rev=git:abc1234";
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <Markdown value={"![logo](img/logo.png)\n\n![web](https://a.test/x.png)\n\n![gone](img/gone.png)\n\nRead [the guide](guide.md).\n"} base={base} />
    </PageNavigationContext.Provider>,
  );
  await settle();
  const logo = await waitFor(() => view.container.querySelector('img[alt="logo"]') as HTMLImageElement);
  expect(logo.getAttribute("src")).toBe("blob:test-image/png");
  expect(reads).toEqual([
    { path: "docs/img/logo.png", at: { rev: "git:abc1234" } },
    { path: "docs/img/gone.png", at: { rev: "git:abc1234" } },
  ]);
  expect((view.container.querySelector('img[alt="web"]') as HTMLImageElement).getAttribute("src")).toBe("https://a.test/x.png");
  // A missing image keeps its alt text, as a browser would.
  expect((view.container.querySelector('img[alt="gone"]') as HTMLImageElement).getAttribute("src")).toBeNull();
  fireEvent.click(view.container.querySelector('a[href="guide.md"]')!);
  expect(opened).toEqual(["oxplow:oxplow/file?path=docs/guide.md&rev=git:abc1234"]);
});
