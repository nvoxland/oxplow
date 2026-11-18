/// The note prompt every Add to Context opens: what is being added, and an
/// optional note of what the person wants the agent to do about it. One for
/// the app (`App` mounts it); `addToContext` opens it. It sits where the
/// person last pressed (the selection's toolbar, a menu item), kept on
/// screen. Ctrl/Cmd+Enter or Add adds — with no note too; Enter is a new
/// line; Escape, Cancel, or a click outside before anything is typed adds
/// nothing (`.context/usability.md` "Enter submits").

import { useEffect, useRef, useState } from "react";

import { formatShortcut } from "../../keybindings.js";
import { cancelAddToContext, submitToContext, usePendingAdd, type ContextWhat } from "../../contextBasket.js";

const WIDTH = 380;
const HEIGHT = 190;

// Where the person last pressed: the prompt opens there.
let lastPress: { x: number; y: number } | null = null;
if (typeof document !== "undefined") {
  const note = (e: MouseEvent) => {
    lastPress = { x: e.clientX, y: e.clientY };
  };
  document.addEventListener("mousedown", note, true);
  document.addEventListener("contextmenu", note, true);
}

/** What the prompt calls the item: its label, else its page, else its text's start. */
export function whatTitle(what: ContextWhat): string {
  return what.label || what.uri || (what.text ?? "").slice(0, 80);
}

export function ContextNotePrompt() {
  const add = usePendingAdd();
  const [note, setNote] = useState("");
  const [at, setAt] = useState<{ left: number; top: number }>({ left: 0, top: 0 });
  const box = useRef<HTMLDivElement | null>(null);
  const field = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    if (!add) return;
    setNote("");
    const x = lastPress?.x ?? window.innerWidth / 2 - WIDTH / 2;
    const y = lastPress?.y ?? 80;
    setAt({
      left: Math.min(Math.max(8, x), window.innerWidth - WIDTH - 8),
      top: Math.min(Math.max(8, y + 8), window.innerHeight - HEIGHT - 8),
    });
    field.current?.focus();
  }, [add]);

  useEffect(() => {
    if (!add) return;
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node) && !field.current?.value.trim()) cancelAddToContext();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [add]);

  if (!add) return null;
  const submit = () => void submitToContext(add, note);
  return (
    <div
      ref={box}
      data-testid="context-note-prompt"
      role="dialog"
      aria-label="Add to Context"
      // Keep the press inside from reaching a host that would take focus back.
      onMouseDown={(e) => e.stopPropagation()}
      style={{
        position: "fixed",
        left: at.left,
        top: at.top,
        zIndex: 1000,
        width: WIDTH,
        display: "flex",
        flexDirection: "column",
        gap: 8,
        padding: 12,
        background: "var(--surface-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: 8,
        boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--text-sm)",
      }}
    >
      <div style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        Add to Context: <span style={{ color: "var(--text-primary)" }}>{whatTitle(add.what)}</span>
      </div>
      <textarea
        ref={field}
        data-testid="context-note-field"
        aria-label="Note"
        placeholder="Note for the agent (optional): what should it do about this?"
        value={note}
        rows={3}
        onChange={(e) => setNote(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            cancelAddToContext();
          }
        }}
        style={{
          resize: "vertical",
          fontFamily: "inherit",
          fontSize: "inherit",
          color: "var(--text-primary)",
          background: "var(--surface-card)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          padding: "6px 8px",
        }}
      />
      <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 8 }}>
        <span style={{ flex: 1, color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>{formatShortcut("Ctrl/Cmd+Enter")} adds</span>
        <button type="button" className="ox-button" data-testid="context-note-cancel" onClick={() => cancelAddToContext()}>
          Cancel
        </button>
        <button type="button" className="ox-button ox-primary" data-testid="context-note-add" onClick={submit}>
          Add
        </button>
      </div>
    </div>
  );
}
