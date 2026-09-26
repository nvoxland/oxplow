import { useEffect, useRef, useState } from "react";
import { NodeViewContent, NodeViewWrapper, ReactNodeViewRenderer } from "@tiptap/react";
import type { NodeViewProps } from "@tiptap/react";
import { CodeBlock } from "@tiptap/extension-code-block";
import { Pencil } from "lucide-react";
import { renderMermaidInto } from "../Wiki/mermaidRender.js";

/**
 * Tiptap node that renders fenced `mermaid` code blocks as live SVG
 * diagrams when the caret isn't inside them. Clicking enters edit
 * mode (raw `<pre><code>` editable surface); Esc or blur returns to
 * the rendered view.
 *
 * Why a custom node instead of intercepting render-time: tiptap-markdown
 * round-trips fenced code blocks as ` ```<lang>\n…\n``` `, so storage
 * stays unchanged. We just teach Tiptap to render the `mermaid`
 * language with a NodeView that owns a hidden editable `<pre>` plus a
 * sibling render host that we paint into via the shared
 * `renderMermaidInto`.
 */
function MermaidNodeView({ node, editor, getPos, selected }: NodeViewProps) {
  const renderRef = useRef<HTMLDivElement | null>(null);
  const [editing, setEditing] = useState(false);
  const cleanupRef = useRef<(() => void) | null>(null);
  const source = node.textContent;

  useEffect(() => {
    if (editing) return;
    const host = renderRef.current;
    if (!host) return;
    let cancelled = false;
    let teardown: (() => void) | null = null;
    void (async () => {
      const fn = await renderMermaidInto(host, source, `editor-${Date.now()}`);
      if (cancelled) {
        fn?.();
        return;
      }
      teardown = fn;
      cleanupRef.current = fn;
    })();
    return () => {
      cancelled = true;
      try { teardown?.(); } catch { /* ignore */ }
      cleanupRef.current = null;
    };
  }, [source, editing]);

  // Auto-exit edit mode when the caret leaves the block. Reading
  // `selected` (Tiptap NodeView prop) tells us whether the node is
  // selected; we also check whether the editor's text selection sits
  // inside our range.
  useEffect(() => {
    if (!editing) return;
    const handler = () => {
      if (typeof getPos !== "function") return;
      const pos = getPos();
      const { from, to } = editor.state.selection;
      const inside = from >= pos && to <= pos + node.nodeSize;
      if (!inside) setEditing(false);
    };
    editor.on("selectionUpdate", handler);
    return () => { editor.off("selectionUpdate", handler); };
  }, [editing, editor, getPos, node.nodeSize]);

  return (
    <NodeViewWrapper
      className="oxplow-rt-block oxplow-rt-mermaid"
      data-editing={editing ? "true" : "false"}
      style={{ position: "relative", margin: "12px 0" }}
    >
      <div
        style={{
          position: "absolute",
          top: 4,
          right: 4,
          zIndex: 2,
          display: "flex",
          gap: 4,
          fontSize: 11,
          color: "var(--text-secondary)",
          background: "var(--surface-elevated)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 4,
          padding: "2px 6px",
          opacity: selected || editing ? 1 : 0.6,
        }}
        contentEditable={false}
      >
        <span>mermaid</span>
        <button
          type="button"
          onMouseDown={(e) => { e.preventDefault(); setEditing((v) => !v); }}
          style={{
            background: "transparent",
            border: "none",
            color: "inherit",
            cursor: "pointer",
            padding: 0,
            display: "inline-flex",
            alignItems: "center",
          }}
          title={editing ? "View diagram" : "Edit source"}
        >
          <Pencil size={12} />
        </button>
      </div>
      {/* Editable source — Tiptap manages this. Hidden when not editing. */}
      <pre
        style={{
          display: editing ? "block" : "none",
          margin: 0,
          padding: "10px 12px",
          background: "var(--surface-card)",
          border: "1px solid var(--border-strong)",
          borderRadius: 6,
          fontFamily: "var(--font-mono)",
          fontSize: "var(--text-sm)",
          color: "var(--text-primary)",
          whiteSpace: "pre-wrap",
        }}
      >
        <NodeViewContent<"code"> as="code" />
      </pre>
      {/* Rendered SVG host — painted imperatively by renderMermaidInto. */}
      <div
        ref={renderRef}
        style={{ display: editing ? "none" : "block", cursor: "pointer" }}
        onClick={() => setEditing(true)}
        contentEditable={false}
      />
    </NodeViewWrapper>
  );
}

export const MermaidBlock = CodeBlock.extend({
  name: "codeBlock",
  addNodeView() {
    return ReactNodeViewRenderer((props) => {
      const lang = (props.node.attrs as { language?: string | null }).language;
      if (lang === "mermaid") {
        return <MermaidNodeView {...props} />;
      }
      // Non-mermaid code blocks: fall through to default <pre><code>.
      return (
        <NodeViewWrapper
          className="oxplow-rt-block oxplow-rt-code"
          style={{ margin: "12px 0" }}
        >
          <pre
            style={{
              margin: 0,
              padding: "10px 12px",
              background: "var(--surface-card)",
              border: "1px solid var(--border-subtle)",
              borderRadius: 6,
              fontFamily: "var(--font-mono)",
              fontSize: "var(--text-sm)",
              color: "var(--text-primary)",
              overflowX: "auto",
            }}
          >
            <NodeViewContent<"code"> as="code" />
          </pre>
        </NodeViewWrapper>
      );
    });
  },
});
