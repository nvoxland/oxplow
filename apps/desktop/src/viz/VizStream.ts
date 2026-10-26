/// The stream a viz page shows its viz for, as a whole `Stream`: what a
/// viz that needs more than its id reads (the editor: the worktree its
/// files are under). Provided by `PageHost`; null outside one.
import { createContext, useContext } from "react";

import type { Stream } from "../api.js";

export const VizStreamContext = createContext<Stream | null>(null);

export function useVizStream(): Stream | null {
  return useContext(VizStreamContext);
}
