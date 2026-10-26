import type { CSSProperties } from "react";

import { DiffPane, type DiffSpec } from "../components/Diff/DiffPane.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { diffTextOf, fileRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

/**
 * `diff`: one file between two revisions, at the height its place gives (a
 * page's whole body on a viz page) — the diff pane `hunks` opens, with its
 * Prev/Next, Ask About This and the `diff.file.header` slot. Open file
 * shows the working file in the editor.
 */
export function DiffViz({ spec, streamId, onOpenPage }: { spec: DiffSpec | null; streamId: string | null; onOpenPage?(ref: TabRef): void }) {
  const nav = useOptionalPageNavigation();
  if (!spec) return null;
  // A diff of literal text (Compare with Clipboard) keeps its two sides in
  // the entry's history state: no address holds them.
  const shown = withDiffText(spec, nav?.historyState);
  const openFile = (path: string) => {
    const ref = fileRef(path);
    if (nav) nav.navigate(ref);
    else onOpenPage?.(ref);
  };
  return (
    <div data-testid="viz-diff" style={style}>
      <DiffPane streamId={streamId ?? ""} spec={shown} visible onJumpToSource={openFile} />
    </div>
  );
}

/** `spec`, its two sides replaced by the literal text its history state
 *  carries (a clipboard compare), if any. */
export function withDiffText(spec: DiffSpec, historyState: Record<string, unknown> | undefined): DiffSpec {
  const text = diffTextOf(historyState);
  return text ? { ...spec, leftContent: text.left, rightContent: text.right } : spec;
}

const style: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 360, height: "100%" };
