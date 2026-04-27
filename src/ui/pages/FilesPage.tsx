import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import { ProjectPanel } from "../components/Panels/ProjectPanel.js";
import type { DiffRequest } from "../components/Diff/diff-request.js";

export interface FilesPageProps {
  stream: Stream | null;
  gitEnabled: boolean;
  selectedFilePath: string | null;
  generatedDirs: string[];
  onOpenFile(path: string): void;
  onOpenDiff?(request: DiffRequest): void;
  onCreateFile(path: string): Promise<void>;
  onCreateDirectory(path: string): Promise<void>;
  onRenamePath(fromPath: string, toPath: string): Promise<void>;
  onDeletePath(path: string): Promise<void>;
  onToggleGeneratedDir(name: string, mark: boolean): Promise<void>;
  commitRequest?: number;
}

/**
 * Thin Page wrapper around the existing ProjectPanel (the file tree +
 * git summary). The legacy left-rail "Files" tool window stays available
 * during the migration.
 */
export function FilesPage(props: FilesPageProps) {
  return (
    <Page testId="page-files" title="Files">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <ProjectPanel {...props} />
      </div>
    </Page>
  );
}
