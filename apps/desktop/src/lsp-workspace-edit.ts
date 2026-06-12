/// Workspace-edit application across open AND non-open files.
///
/// Open files (a Monaco model exists for the uri) are edited through
/// `pushEditOperations` — the change lands in the draft buffer with
/// undo intact, and the user saves as usual. Non-open files are
/// read-modify-written through the workspace file IPC, landing
/// directly on disk. Used by the rename/code-action providers and by
/// server-initiated `workspace/applyEdit` requests.

import { readFile as apiReadFile, writeWorkspaceFile, type Stream } from "./api.js";
import { DISK } from "./file-version.js";
import {
  applyTextEditsToContent,
  toMonacoRange,
  type NormalizedFileEdits,
  type NormalizedWorkspaceEdit,
} from "./lsp-monaco-mapping.js";
import { relativePathFromFileUri } from "./lsp.js";

/* eslint-disable @typescript-eslint/no-explicit-any */

/// Everything the applier needs from the environment, injected so the
/// logic is testable without Monaco or a backend.
export interface WorkspaceEditIO {
  /// The open Monaco model for `uri`, or null when the file isn't open.
  findModel(uri: string): any | null;
  /// file:// uri → repo-relative path inside the stream worktree, or
  /// null when the uri points outside it.
  pathFromUri(uri: string): string | null;
  readFile(path: string): Promise<string>;
  writeFile(path: string, content: string): Promise<void>;
}

export interface ApplyWorkspaceEditResult {
  appliedFiles: number;
  /// Human-readable, per-file failure messages.
  failures: string[];
}

/// The production IO: Monaco's model registry for open files, the
/// workspace file IPC (disk truth) for everything else.
export function workspaceEditIOForStream(monaco: any, stream: Stream): WorkspaceEditIO {
  return {
    findModel: (uri) => monaco.editor.getModel(monaco.Uri.parse(uri)) ?? null,
    pathFromUri: (uri) => relativePathFromFileUri(stream, uri),
    readFile: async (path) => {
      const content = await apiReadFile(stream.id, path, DISK);
      if (content == null) throw new Error("file not found in worktree");
      return content;
    },
    writeFile: async (path, content) => {
      await writeWorkspaceFile(stream.id, path, content);
    },
  };
}

export function partitionByOpenModel(
  io: Pick<WorkspaceEditIO, "findModel">,
  files: NormalizedFileEdits[],
): { open: NormalizedFileEdits[]; closed: NormalizedFileEdits[] } {
  const open: NormalizedFileEdits[] = [];
  const closed: NormalizedFileEdits[] = [];
  for (const file of files) {
    (io.findModel(file.uri) ? open : closed).push(file);
  }
  return { open, closed };
}

export async function applyNormalizedWorkspaceEdit(
  monaco: any,
  io: WorkspaceEditIO,
  normalized: NormalizedWorkspaceEdit,
): Promise<ApplyWorkspaceEditResult> {
  let appliedFiles = 0;
  const failures: string[] = [];
  for (const file of normalized.files) {
    try {
      const model = io.findModel(file.uri);
      if (model) {
        model.pushEditOperations(
          [],
          file.edits.map((edit) => ({
            range: toMonacoRange(monaco, edit.range),
            text: edit.newText,
          })),
          () => null,
        );
      } else {
        const path = io.pathFromUri(file.uri);
        if (path == null) {
          throw new Error("outside the stream worktree");
        }
        const content = await io.readFile(path);
        await io.writeFile(path, applyTextEditsToContent(content, file.edits));
      }
      appliedFiles += 1;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      failures.push(`${file.uri}: ${message}`);
    }
  }
  return { appliedFiles, failures };
}
