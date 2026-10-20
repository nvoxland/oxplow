import { Page } from "../tabs/Page.js";
import { DiffPane, type DiffSpec } from "../components/Diff/DiffPane.js";
import type { Stream } from "../tauri-bridge/index.js";
import { useOptionalPageNavigation, usePageTitle } from "../tabs/PageNavigationContext.js";
import { diffTextOf } from "../tabs/pageRefs.js";

export interface DiffPageProps {
  stream: Stream;
  spec: DiffSpec;
  visible: boolean;
  onJumpToSource(path: string): void;
}

/**
 * Thin Page wrapper around `DiffPane` so diff tabs share the same
 * browser-style chrome (title row, optional nav bar) as every other
 * non-agent tab. The title comes from the diff spec via
 * `usePageTitle`. A diff of literal text (a clipboard compare) reads its
 * two sides from the entry's history state (`diffTextOf`).
 */
export function DiffPage({ stream, spec: versions, visible, onJumpToSource }: DiffPageProps) {
  const text = diffTextOf(useOptionalPageNavigation()?.historyState);
  const spec = text ? { ...versions, leftContent: text.left, rightContent: text.right } : versions;
  const basename = spec.path.split("/").pop() ?? spec.path;
  const suffix = spec.labelOverride ?? "diff";
  usePageTitle(`${basename} (${suffix})`);
  return (
    <Page testId="page-diff" kind="diff">
      <DiffPane
        streamId={stream.id}
        spec={spec}
        visible={visible}
        onJumpToSource={onJumpToSource}
      />
    </Page>
  );
}
