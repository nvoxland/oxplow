import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { VizConfig, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";

// `media` shows each row's file as what it is: an image in an <img>, a PDF
// in a frame, from the file's bytes on its filesystem at its revision. A
// missing file says so and is never read; so does one that is neither.

const reads: { path: string; revision: string | undefined; filesystem?: string }[] = [];
const urls: string[] = [];
let revoked = 0;

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
mock.module("../api.js", () => ({
  ...realApi,
  statFile: async (_stream: string, path: string) => {
    if (path === "gone.png") return null;
    if (path === "huge.png") return { kind: "file", size: 40_000_000, modifiedAt: null, isText: false, language: null };
    return { kind: "file", size: 2048, modifiedAt: null, isText: false, language: null };
  },
  readBytesAt: async (_stream: string, path: string, at: { rev?: string; filesystem?: string }) => {
    reads.push({ path, revision: at.rev, ...(at.filesystem ? { filesystem: at.filesystem } : {}) });
    return new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
  },
}));
const { VizView } = await import("./VizView.js");

const realCreate = URL.createObjectURL;
const realRevoke = URL.revokeObjectURL;
URL.createObjectURL = (blob: Blob) => {
  const url = `blob:test-${urls.length}-${blob.type}`;
  urls.push(url);
  return url;
};
URL.revokeObjectURL = () => {
  revoked += 1;
};

afterAll(() => {
  mock.module("../api.js", () => pristineApi);
  URL.createObjectURL = realCreate;
  URL.revokeObjectURL = realRevoke;
});
afterEach(() => {
  cleanup();
  reads.length = 0;
  urls.length = 0;
  revoked = 0;
});

const base = {
  id: "x/m",
  extension: "x",
  slug: "m",
  title: "M",
  description: "",
  query: "",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  code: null,
  media: null,
  diff: null,
  markdown: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  parts: [],
  launcherCategory: null,
  actions: [],
  alert: null,
  needs: [],
  fills: true,
  origin: { extension: "x", source: null, running: "", deployment: null, bundled: false },
  tool: null,
};

function run(viz: Partial<VizConfig>, columns: string[], rows: SqlCell[][]): VizRun {
  return {
    config: { ...base, ...viz },
    params: {},
    result: { columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} },
  } as unknown as VizRun;
}

function nav(navigated: TabRef[]): PageNavigation {
  return { navigate: (to) => navigated.push(to), goBack() {}, goForward() {}, canGoBack: false, canGoForward: false };
}

test("a media viz shows an image, frames a PDF, and says what it can't show", async () => {
  const navigated: TabRef[] = [];
  const view = render(
    <PageNavigationContext.Provider value={nav(navigated)}>
      <VizView
        run={run({ viz: "media", media: { path: "p", filesystem: "f", revision: "r" } }, ["p", "f", "r"], [
          ["img/logo.png", null, "git:abc1234"],
          ["docs/spec.pdf", "oxplow:temp", null],
          ["gone.png", null, "working"],
          ["huge.png", null, "working"],
          ["notes.txt", null, "working"],
        ])}
        streamId="str1"
        onOpenPage={() => {}}
      />
    </PageNavigationContext.Provider>,
  );
  // The missing, too-large and unshowable files are never read.
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(reads).toEqual([
    { path: "img/logo.png", revision: "git:abc1234" },
    { path: "docs/spec.pdf", filesystem: "oxplow:temp", revision: undefined },
  ]);
  const files = view.getAllByTestId("viz-media-file");
  expect(files.map((f) => f.querySelector("[data-testid=viz-media-path]")!.textContent)).toEqual(["img/logo.png", "docs/spec.pdf", "gone.png", "huge.png", "notes.txt"]);
  expect(files[0]!.textContent).toContain("@abc1234");
  const image = await waitFor(() => view.getByTestId("viz-media-image") as HTMLImageElement);
  expect(image.getAttribute("src")).toBe("blob:test-0-image/png");
  expect(image.getAttribute("alt")).toBe("img/logo.png");
  const pdf = await waitFor(() => view.getByTestId("viz-media-pdf") as HTMLIFrameElement);
  expect(pdf.getAttribute("src")).toBe("blob:test-1-application/pdf");
  const problems = view.getAllByTestId("viz-media-problem").map((p) => p.textContent);
  expect(problems).toEqual(["Not there at working tree", "Too large to show, 40000000 bytes", "Not an image or a PDF"]);
  fireEvent.click(files[0]!.querySelector("[data-testid=viz-media-open]")!);
  expect(navigated.map((r) => r.id)).toEqual(["oxplow:oxplow/file?path=img/logo.png"]);
  view.unmount();
  // The blob URLs are given back when the viz goes.
  expect(revoked).toBe(2);
});

test("a media viz that is the page's own view has no Open file", async () => {
  const vizRun = run({ viz: "media", media: { path: "p", filesystem: null, revision: null } }, ["p"], [["img/logo.png"]]);
  const page = render(<VizView run={vizRun} streamId="str1" onOpenPage={() => {}} isPage />);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(page.queryByTestId("viz-media-open")).toBeNull();
  expect(page.getByTestId("viz-media-file").textContent).toContain("2 KB");
});
