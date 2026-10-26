/// The stream a lens page shows its lens for, as a whole `Stream`: what a
/// viz that needs more than its id reads (the editor: the worktree its
/// files are under). Provided by `PageHost`; null outside one.
import { createContext, useContext } from "react";

import type { Stream } from "../api.js";

export const LensStreamContext = createContext<Stream | null>(null);

export function useLensStream(): Stream | null {
  return useContext(LensStreamContext);
}
