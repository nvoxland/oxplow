import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import { CodeQualityPanel } from "../components/CodeQuality/CodeQualityPanel.js";

export interface CodeQualityPageProps {
  stream: Stream | null;
  onOpenFile?: (path: string) => void;
}

/**
 * Thin Page wrapper around the existing CodeQualityPanel. The panel's UI
 * stays put — this exists so the rail HUD can route to "Code quality" as a
 * full-area tab instead of a bottom-drawer pane. The panel will get a
 * density / web-style-interaction polish pass in later phases.
 */
export function CodeQualityPage({ stream, onOpenFile }: CodeQualityPageProps) {
  return (
    <Page
      testId="page-code-quality"
      title="Code quality"
    >
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <CodeQualityPanel stream={stream} onOpenFile={onOpenFile} />
      </div>
    </Page>
  );
}
