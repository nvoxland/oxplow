export interface EditorFocusSelection {
  startLine: number;
  startColumn: number;
  endLine: number;
  endColumn: number;
  text: string;
}

export interface EditorFocusOpenFile {
  path: string;
  dirty: boolean;
}

export interface EditorFocusState {
  activeFile: string | null;
  caret: { line: number; column: number } | null;
  selection: EditorFocusSelection | null;
  openFiles: EditorFocusOpenFile[];
  updatedAt: string;
}

export class EditorFocusStore {
  private states = new Map<string, EditorFocusState>();

  get(streamId: string): EditorFocusState | null {
    return this.states.get(streamId) ?? null;
  }

  set(streamId: string, state: EditorFocusState): void {
    this.states.set(streamId, state);
  }

  clear(streamId: string): void {
    this.states.delete(streamId);
  }
}

const SELECTION_MAX_LINES = 200;
const OPEN_FILES_MAX = 30;

export function formatEditorFocusForAgent(state: EditorFocusState | null): string | null {
  if (!state) return null;
  const sections: string[] = [];
  if (state.activeFile) sections.push(`Active file: ${state.activeFile}`);
  if (state.selection) {
    sections.push(formatSelection(state.activeFile, state.selection));
  } else if (state.caret) {
    sections.push(`Caret: line ${state.caret.line}, col ${state.caret.column}`);
  }
  if (state.openFiles.length > 0) {
    sections.push(formatOpenFiles(state.openFiles));
  }
  if (sections.length === 0) return null;
  return `<editor-context>\n${sections.join("\n")}\n</editor-context>`;
}

function formatSelection(activeFile: string | null, selection: EditorFocusSelection): string {
  const label = activeFile
    ? `Selection (${activeFile}:${selection.startLine}-${selection.endLine}):`
    : `Selection (lines ${selection.startLine}-${selection.endLine}):`;
  const rawLines = selection.text.split("\n");
  let body: string;
  if (rawLines.length > SELECTION_MAX_LINES) {
    const remaining = rawLines.length - SELECTION_MAX_LINES;
    body = `${rawLines.slice(0, SELECTION_MAX_LINES).join("\n")}\n… [truncated ${remaining} more lines]`;
  } else {
    body = selection.text;
  }
  return `${label}\n\`\`\`\n${body}\n\`\`\``;
}

function formatOpenFiles(files: EditorFocusOpenFile[]): string {
  const shown = files.slice(0, OPEN_FILES_MAX);
  const remaining = files.length - shown.length;
  const rendered = shown
    .map((file) => (file.dirty ? `${file.path} (dirty)` : file.path))
    .join(", ");
  return remaining > 0
    ? `Open tabs: ${rendered} (+${remaining} more)`
    : `Open tabs: ${rendered}`;
}
