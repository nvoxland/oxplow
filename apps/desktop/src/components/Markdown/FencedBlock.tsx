import { useEffect, useRef, useState } from "react";
import { NodeViewContent, NodeViewWrapper, ReactNodeViewRenderer } from "@tiptap/react";
import type { NodeViewProps } from "@tiptap/react";
import { CodeBlockLowlight } from "@tiptap/extension-code-block-lowlight";
import { Pencil } from "lucide-react";
import { renderMermaidInto } from "./mermaidRender.js";
import { InlineView } from "./InlineView.js";
import { CopyCodeButton } from "./CopyCodeButton.js";
import { lowlight } from "./codeHighlight.js";

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

/** A live page remains a non-editable island until its YAML is opened.
 * The editable content is still Tiptap's codeBlock, so its fence survives. */
function PageNodeView({ node, editor, getPos }: NodeViewProps) {
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) return;
    // ProseMirror owns events from NodeViewContent and can consume them
    // before the React node-view's handler. Capture Escape on the editor
    // DOM itself, before ProseMirror's bubbling keyboard handlers.
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const pos = getPos();
      if (pos === undefined) return;
      const { from, to } = editor.state.selection;
      if (from < pos || to > pos + node.nodeSize) return;
      event.preventDefault();
      event.stopPropagation();
      setEditing(false);
    };
    const dom = editor.view.dom;
    dom.addEventListener("keydown", escape, true);
    return () => dom.removeEventListener("keydown", escape, true);
  }, [editing, editor, getPos, node.nodeSize]);
  useEffect(() => {
    if (!editing) return;
    const leave = () => {
      const pos = getPos();
      if (pos === undefined) return;
      const { from, to } = editor.state.selection;
      if (from < pos || to > pos + node.nodeSize) setEditing(false);
    };
    editor.on("selectionUpdate", leave);
    return () => { editor.off("selectionUpdate", leave); };
  }, [editing, editor, getPos, node.nodeSize]);
  return (
    <NodeViewWrapper
      className="oxplow-rt-block oxplow-rt-page"
      data-editing={editing ? "true" : "false"}
      style={{ position: "relative", margin: "12px 0" }}
      onKeyDownCapture={(event) => {
        if (editing && event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          setEditing(false);
        }
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as globalThis.Node | null)) setEditing(false);
      }}
    >
      <div contentEditable={false} style={{ display: "flex", justifyContent: "flex-end" }}>
        <button type="button" className="ox-button" title={editing ? "View page" : "Edit YAML"}
          onMouseDown={(event) => {
            event.preventDefault();
            const pos = getPos();
            if (!editing && pos !== undefined) editor.chain().focus().setTextSelection(pos + 1).run();
            setEditing(!editing);
          }}>
          <Pencil size={12} /> {editing ? "View page" : "Edit YAML"}
        </button>
      </div>
      <pre style={{ display: editing ? "block" : "none", whiteSpace: "pre-wrap" }}>
        <NodeViewContent<"code"> as="code" />
      </pre>
      {!editing && <div contentEditable={false}><InlineView specYaml={node.textContent} /></div>}
    </NodeViewWrapper>
  );
}

// Highlighted as the viewer highlights (`codeHighlight.ts`): lowlight's
// decorations over the editable text, so the stored fence is unchanged.
export const FencedBlock = CodeBlockLowlight.configure({ lowlight }).extend({
  name: "codeBlock",
  addNodeView() {
    return ReactNodeViewRenderer((props) => {
      const lang = (props.node.attrs as { language?: string | null }).language;
      if (lang === "mermaid") {
        return <MermaidNodeView {...props} />;
      }
      if (lang === "oxplow-page") return <PageNodeView {...props} />;
      // Any other code block: its source, with the viewer's Copy.
      return (
        <NodeViewWrapper
          className="oxplow-rt-block oxplow-rt-code oxplow-md-code"
          style={{ margin: "12px 0" }}
        >
          <CopyCodeButton read={() => props.node.textContent} />
          {/* Drawn as the viewer draws a code block (`.oxplow-md pre`). */}
          <pre>
            <NodeViewContent<"code"> as="code" />
          </pre>
        </NodeViewWrapper>
      );
    });
  },
});
