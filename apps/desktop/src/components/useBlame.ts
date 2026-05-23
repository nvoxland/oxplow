//! Blame-overlay state for `EditorPane`, extracted into a hook.
//!
//! Owns the merged local+git blame entries, the gutter scroll/line-height
//! sync that keeps the `BlameOverlay` aligned with the editor, and the
//! refresh-on-save behavior. `EditorPane` renders the overlay from the
//! returned state and wires "Annotate with Blame" to `toggleBlame`.
//!
//! Behavior-preserving extraction — effect bodies + dependency arrays are
//! unchanged, just relocated. (No automated net: Monaco doesn't mount
//! under happy-dom.)

import {
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from "react";

import { localBlame, type LocalBlameEntry, type Stream } from "../api.js";

/// Left-gutter width (px) reserved for the blame overlay while it's on.
export const BLAME_WIDTH = 150;

export interface BlameState {
  blame: { path: string; entries: LocalBlameEntry[] } | null;
  blameScrollTop: number;
  blameLineHeight: number;
  /// Toggle the overlay for the current file (fetch on, clear off).
  toggleBlame: () => void;
}

/* eslint-disable @typescript-eslint/no-explicit-any */
export function useBlame(opts: {
  editorRef: MutableRefObject<any>;
  monacoRef: MutableRefObject<any>;
  monacoReady: boolean;
  streamRef: MutableRefObject<Stream>;
  filePathRef: MutableRefObject<string | null>;
  filePath: string | null;
  isDirty: boolean;
  setLspStatus: Dispatch<SetStateAction<string | null>>;
}): BlameState {
  const { editorRef, monacoRef, monacoReady, streamRef, filePathRef, filePath, isDirty, setLspStatus } =
    opts;

  const [blame, setBlame] = useState<{ path: string; entries: LocalBlameEntry[] } | null>(null);
  const [blameScrollTop, setBlameScrollTop] = useState(0);
  const [blameLineHeight, setBlameLineHeight] = useState(19);
  const prevDirtyRef = useRef(isDirty);

  async function refreshBlame(path: string) {
    try {
      const entries = await localBlame(streamRef.current.id, path);
      if (filePathRef.current !== path) return;
      if (entries.length === 0) {
        setBlame(null);
        setLspStatus("No blame available");
        setTimeout(() => setLspStatus((s) => (s === "No blame available" ? null : s)), 2500);
        return;
      }
      setBlame({ path, entries });
    } catch (err) {
      setLspStatus(`Blame failed: ${String(err)}`);
    }
  }

  function toggleBlame() {
    if (!filePath) return;
    if (blame && blame.path === filePath) {
      setBlame(null);
      return;
    }
    void refreshBlame(filePath);
  }

  // Drop the overlay when the editor switches to a different file.
  useEffect(() => {
    if (blame && blame.path !== filePath) setBlame(null);
  }, [filePath, blame]);

  // Re-fetch on save (isDirty true→false) so attribution tracks the new
  // closed effort / HEAD. Not on every edit — attribution is relative to
  // the last commit, not the buffer.
  useEffect(() => {
    const wasDirty = prevDirtyRef.current;
    prevDirtyRef.current = isDirty;
    if (!blame || blame.path !== filePath) return;
    if (wasDirty && !isDirty) {
      void refreshBlame(filePath);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isDirty, filePath, blame]);

  // While the overlay is on, widen the left gutter and mirror the editor's
  // scroll + line-height so the rows stay aligned.
  useEffect(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (!editor || !monaco) return;
    if (blame) {
      editor.updateOptions({ lineNumbers: "off", lineDecorationsWidth: BLAME_WIDTH });
      setBlameLineHeight(editor.getOption(monaco.editor.EditorOption.lineHeight));
      setBlameScrollTop(editor.getScrollTop());
      const d = editor.onDidScrollChange((e: any) => setBlameScrollTop(e.scrollTop));
      return () => {
        d.dispose();
        editor.updateOptions({ lineNumbers: "on", lineDecorationsWidth: 10 });
      };
    }
    return undefined;
  }, [blame, monacoReady]);

  return { blame, blameScrollTop, blameLineHeight, toggleBlame };
}
