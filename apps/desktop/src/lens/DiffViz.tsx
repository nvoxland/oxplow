import type { CSSProperties } from "react";

import { DiffPane, type DiffSpec } from "../components/Diff/DiffPane.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { fileRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

/**
 * `diff`: one file between two revisions, at the height its place gives (a
 * page's whole body on a lens page) — the diff pane `hunks` opens, with its
 * Prev/Next, Ask About This and the `diff.file.header` slot. Open file
 * shows the working file in the editor.
 */
export function DiffViz({ spec, streamId, onOpenPage }: { spec: DiffSpec | null; streamId: string | null; onOpenPage?(ref: TabRef): void }) {
  const nav = useOptionalPageNavigation();
  if (!spec) return null;
  const openFile = (path: string) => {
    const ref = fileRef(path);
    if (nav) nav.navigate(ref);
    else onOpenPage?.(ref);
  };
  return (
    <div data-testid="lens-diff" style={style}>
      <DiffPane streamId={streamId ?? ""} spec={spec} visible onJumpToSource={openFile} />
    </div>
  );
}

const style: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 360, height: "100%" };
