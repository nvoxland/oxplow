import "monaco-editor/min/vs/editor/editor.main.css";
import "@xterm/xterm/css/xterm.css";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import CssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
import HtmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
import TsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";
import { createRoot } from "react-dom/client";
import { Root } from "./Root.js";
import { installUiLogging, logUi } from "./logger.js";
import { remoteBaseUrl, windowKind } from "./tauri-bridge/transport.js";

self.MonacoEnvironment = {
  getWorker(_workerId: string, label: string) {
    switch (label) {
      case "json":
        return new JsonWorker();
      case "css":
      case "scss":
      case "less":
        return new CssWorker();
      case "html":
      case "handlebars":
      case "razor":
        return new HtmlWorker();
      case "typescript":
      case "javascript":
        return new TsWorker();
      default:
        return new EditorWorker();
    }
  },
};

installUiLogging();
// Which window this is and which backend it resolved — the first thing
// you want when a window misbehaves, and the only visible proof that
// the shell's injected context arrived.
logUi("info", "ui bootstrapping", {
  windowKind: windowKind(),
  daemon: remoteBaseUrl(),
});

const el = document.getElementById("root")!;
createRoot(el).render(<Root />);
