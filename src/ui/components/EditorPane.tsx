import { useEffect, useRef } from "react";
import type { Stream } from "../api.js";

export function EditorPane({ stream }: { stream: Stream }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const disposeRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      const editor = monaco.editor.create(hostRef.current, {
        value: [
          `// stream: ${stream.title}`,
          `// branch: ${stream.branch}`,
          `// worktree: ${stream.worktree_path}`,
          "",
          "// editor not wired up yet",
          "",
        ].join("\n"),
        language: "typescript",
        theme: "vs-dark",
        automaticLayout: true,
        minimap: { enabled: false },
      });
      disposeRef.current = () => editor.dispose();
    })();
    return () => {
      cancelled = true;
      disposeRef.current?.();
    };
  }, [stream]);

  return <div ref={hostRef} style={{ width: "100%", height: "100%" }} />;
}
