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
import { setRecordStores } from "./src/recordStores.js";
setRecordStores([
  { model: "oxplow/knowledge_record", store: "oxwiki", resource: "oxplow/knowledge", storeParam: "store", key: "id", idPattern: "[a-z0-9]+(-[a-z0-9]+)*", loose: false },
  { model: "oxplow/task_record", store: "oxtask", resource: "oxplow/task", storeParam: "provider", key: "id", idPattern: "tsk[0-9]+", loose: true },
]);
