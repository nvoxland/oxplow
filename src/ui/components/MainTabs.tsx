import type { ReactNode } from "react";
import type { Stream, Batch } from "../api.js";
import type { OpenFileState } from "../../file-session.js";
import type { EditorNavigationTarget } from "../lsp.js";
import { TerminalPane } from "./TerminalPane.js";
import { EditorPane } from "./EditorPane.js";

export type TabId = "agent" | "editor";

interface Props {
  stream: Stream;
  batch: Batch | null;
  activeBatchId: string | null;
  active: TabId;
  onActiveChange(tab: TabId): void;
  openFileOrder: string[];
  openFiles: Record<string, OpenFileState>;
  currentFilePath: string | null;
  currentFileContent: string;
  currentFileDirty: boolean;
  onEditorChange(value: string): void;
  onEditorSave(): void;
  editorFindRequest: number;
  editorNavigationTarget: EditorNavigationTarget | null;
  onNavigateToLocation(target: EditorNavigationTarget): Promise<void>;
  onSelectOpenFile(path: string): void;
  onCloseOpenFile(path: string): void;
}

export function MainTabs({
  stream,
  batch,
  activeBatchId,
  active,
  onActiveChange,
  openFileOrder,
  openFiles,
  currentFilePath,
  currentFileContent,
  currentFileDirty,
  onEditorChange,
  onEditorSave,
  editorFindRequest,
  editorNavigationTarget,
  onNavigateToLocation,
  onSelectOpenFile,
  onCloseOpenFile,
}: Props) {
  const tabs: { id: TabId; label: string }[] = [
    { id: "agent", label: batch ? batch.title : "Agent" },
    { id: "editor", label: "Editor" },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-2)" }}>
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => onActiveChange(t.id)}
            style={{
              padding: "8px 16px",
              background: active === t.id ? "var(--bg)" : "transparent",
              color: active === t.id ? "var(--fg)" : "var(--muted)",
              border: "none",
              borderRight: "1px solid var(--border)",
              borderBottom: active === t.id ? "2px solid var(--accent)" : "2px solid transparent",
              cursor: "pointer",
              fontFamily: "inherit",
              fontSize: 12,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, position: "relative" }}>
        <PaneHost visible={active === "agent"}>
          {batch ? (
            <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
              <div style={{ padding: "6px 10px", borderBottom: "1px solid var(--border)", color: "var(--muted)", fontSize: 11 }}>
                {batch.id === activeBatchId
                  ? "Active batch — edits in this session affect the stream worktree."
                  : "Queued batch — use this session for planning and questions before promotion."}
              </div>
              <div style={{ flex: 1, minHeight: 0 }}>
                <TerminalPane paneTarget={batch.pane_target} visible={active === "agent"} />
              </div>
            </div>
          ) : (
            <div style={{ padding: 12, color: "var(--muted)" }}>No batch selected.</div>
          )}
        </PaneHost>
        <PaneHost visible={active === "editor"}>
          <EditorPane
            stream={stream}
            filePath={currentFilePath}
            value={currentFileContent}
            isDirty={currentFileDirty}
            onChange={onEditorChange}
            onSave={onEditorSave}
            findRequest={editorFindRequest}
            navigationTarget={editorNavigationTarget}
            onNavigateToLocation={onNavigateToLocation}
            openFileOrder={openFileOrder}
            openFiles={openFiles}
            onSelectOpenFile={onSelectOpenFile}
            onCloseOpenFile={onCloseOpenFile}
          />
        </PaneHost>
      </div>
    </div>
  );
}

function PaneHost({ visible, children }: { visible: boolean; children: ReactNode }) {
  return (
    <div
      style={{
        position: "absolute",
        inset: 0,
        display: visible ? "block" : "none",
      }}
    >
      {children}
    </div>
  );
}
