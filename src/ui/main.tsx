import { createRoot } from "react-dom/client";
import { App } from "./App.js";

const el = document.getElementById("root")!;
createRoot(el).render(<App />);
