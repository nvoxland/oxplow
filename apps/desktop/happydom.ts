// Test preload: register happy-dom globals (window, document, location,
// sessionStorage, …) so component tests can render React into a real
// DOM. Bun's test runtime has none by default. Wired via bunfig.toml's
// `[test] preload`. Pure-logic tests are unaffected — they just gain a
// DOM they don't use.
import { GlobalRegistrator } from "@happy-dom/global-registrator";

// A frame's page is never fetched: tests render frames (a custom
// component's) without a server behind them.
GlobalRegistrator.register({ settings: { disableIframePageLoading: true } });

// The test world has oxwiki's and oxtask's stores, as the app does once
// their extensions are on: a bare `[[some-page]]` is one of oxwiki's pages
// and `[[tsk42]]` a task (each fill's id pattern).
// Every test starts in that world: a test (or a loader it mounts) that
// changes the stores can't leak them into the files that run after it,
// whatever order the runner picks.
import { afterAll, beforeEach, mock } from "bun:test";
import { dirname } from "node:path";
import { setRecordStores, type RecordStore } from "./src/recordStores.js";
const TEST_STORES: readonly RecordStore[] = [
  { model: "oxplow/knowledge_record", store: "oxwiki", resource: "oxplow/knowledge", storeParam: "store", key: "id", idPattern: "[a-z0-9]+(-[a-z0-9]+)*", loose: false },
  { model: "oxplow/task_record", store: "oxtask", resource: "oxplow/task", storeParam: "provider", key: "id", idPattern: "tsk[0-9]+", loose: true },
];
setRecordStores(TEST_STORES);
beforeEach(() => setRecordStores(TEST_STORES));

// `mock.module` is process-wide and patches a loaded module in place, so a
// file that mocks a module and doesn't put it back (or copies it after
// another file mocked it) leaks into every file after it, in whatever order
// the runner picks. `mock.module` is wrapped here: the first time any file
// mocks a module its pristine exports are kept, and the mocking file gets an
// `afterAll` that puts them back. A file needs no restore of its own.
const pristineModules = new Map<string, Record<string, unknown>>();
const realMockModule = mock.module.bind(mock);
const callerDir = (): string => {
  for (const line of new Error().stack?.split("\n").slice(1) ?? []) {
    const m = /\(?((?:file:\/\/)?\/[^():]+?):\d+:\d+\)?$/.exec(line.trim());
    if (m && !m[1]!.endsWith("/happydom.ts")) return dirname(m[1]!.replace(/^file:\/\//, ""));
  }
  return process.cwd();
};
mock.module = ((spec: string, factory: () => unknown) => {
  let resolved: string | null = null;
  try {
    resolved = Bun.resolveSync(spec, callerDir());
  } catch {
    // Not resolvable here: mocked as asked, with no automatic restore.
  }
  if (resolved !== null) {
    const path = resolved;
    if (!pristineModules.has(path)) {
      try {
        pristineModules.set(path, { ...(require(path) as Record<string, unknown>) });
      } catch {
        // A module that can't be loaded synchronously is left to its file.
      }
    }
    const pristine = pristineModules.get(path);
    if (pristine) {
      // Block body: a hook that returns what `mock.module` returns breaks the next file's mocks.
      afterAll(() => {
        realMockModule(path, () => pristine);
      });
    }
  }
  return realMockModule(spec, factory as never);
}) as typeof mock.module;

// Process-wide caches start empty in every test.
import { clearRefTitles } from "./src/refTitleCache.js";
import { resetAppRememberedStore } from "./src/prefs/remembered.js";
beforeEach(() => {
  clearRefTitles();
  resetAppRememberedStore();
});
