// Test preload: register happy-dom globals (window, document, location,
// sessionStorage, …) so component tests can render React into a real
// DOM. Bun's test runtime has none by default. Wired via bunfig.toml's
// `[test] preload`. Pure-logic tests are unaffected — they just gain a
// DOM they don't use.
import { GlobalRegistrator } from "@happy-dom/global-registrator";

// A frame's page is never fetched: tests render frames (a custom
// component's) without a server behind them.
GlobalRegistrator.register({ settings: { disableIframePageLoading: true } });
