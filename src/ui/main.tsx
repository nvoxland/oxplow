import "monaco-editor/min/vs/editor/editor.main.css";
import "@xterm/xterm/css/xterm.css";
import { createRoot } from "react-dom/client";
import { App } from "./App.js";
import { installUiLogging, logUi } from "./logger.js";
import { initTheme } from "./theme.js";

installUiLogging();
initTheme();
logUi("info", "ui bootstrapping");

const el = document.getElementById("root")!;
createRoot(el).render(<App />);
