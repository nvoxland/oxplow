import { createRoot } from "react-dom/client";
import { App } from "./App.js";
import { installUiLogging, logUi } from "./logger.js";

installUiLogging();
logUi("info", "ui bootstrapping");

const el = document.getElementById("root")!;
createRoot(el).render(<App />);
