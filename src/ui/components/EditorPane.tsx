import { useEffect, useRef } from "react";

export function EditorPane() {
  const hostRef = useRef<HTMLDivElement>(null);
  const disposeRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      const editor = monaco.editor.create(hostRef.current, {
        value: "// editor not wired up yet\n",
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
  }, []);

  return <div ref={hostRef} style={{ width: "100%", height: "100%" }} />;
}
