import { Page } from "../tabs/Page.js";
import { DiffPane, type DiffSpec } from "../components/Diff/DiffPane.js";
import type { Stream } from "../api.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";

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
 * `usePageTitle`.
 */
export function DiffPage({ stream, spec, visible, onJumpToSource }: DiffPageProps) {
  const basename = spec.path.split("/").pop() ?? spec.path;
  const suffix = spec.labelOverride ?? "diff";
  usePageTitle(`${basename} (${suffix})`);
  return (
    <Page testId="page-diff" kind="diff">
      <DiffPane
        stream={stream}
        spec={spec}
        visible={visible}
        onJumpToSource={onJumpToSource}
      />
    </Page>
  );
}
