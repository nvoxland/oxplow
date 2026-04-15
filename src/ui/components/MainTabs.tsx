import type { OpenFileState } from "../../file-session.js";
import type { Stream } from "../api.js";
import { TerminalPane } from "./TerminalPane.js";
import { EditorPane } from "./EditorPane.js";

export type TabId = "working" | "talking" | "editor";

interface Props {
  stream: Stream;
  active: TabId;
  onActiveChange(tab: TabId): void;
  openFileOrder: string[];
  openFiles: Record<string, OpenFileState>;
  currentFilePath: string | null;
  currentFileContent: string;
  currentFileDirty: boolean;
  currentFileLoading: boolean;
  onEditorChange(value: string): void;
  onEditorSave(): void;
  onSelectOpenFile(path: string): void;
  onCloseOpenFile(path: string): void;
}

export function MainTabs({
  stream,
  active,
  onActiveChange,
  openFileOrder,
  openFiles,
  currentFilePath,
  currentFileContent,
  currentFileDirty,
  currentFileLoading,
  onEditorChange,
  onEditorSave,
  onSelectOpenFile,
  onCloseOpenFile,
}: Props) {

  const tabs: { id: TabId; label: string }[] = [
    { id: "working", label: "Working CC" },
    { id: "talking", label: "Talking CC" },
    { id: "editor", label: currentFilePath ? fileLabel(currentFilePath, currentFileDirty) : "Editor" },
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
        <PaneHost visible={active === "working"}>
          <TerminalPane paneTarget={stream.panes.working} />
        </PaneHost>
        <PaneHost visible={active === "talking"}>
          <TerminalPane paneTarget={stream.panes.talking} />
        </PaneHost>
        <PaneHost visible={active === "editor"}>
          <EditorPane
            stream={stream}
            filePath={currentFilePath}
            value={currentFileContent}
            isDirty={currentFileDirty}
            isLoading={currentFileLoading}
            onChange={onEditorChange}
            onSave={onEditorSave}
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

function fileLabel(path: string, isDirty: boolean): string {
  const parts = path.split("/");
  const name = parts[parts.length - 1] ?? path;
  return isDirty ? `${name} •` : name;
}

function PaneHost({ visible, children }: { visible: boolean; children: React.ReactNode }) {
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
