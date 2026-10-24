import { AtSign, MessageSquarePlus } from "lucide-react";

/// A small floating "Add comment" button shown over a fresh text
/// selection on a commentable surface. Non-destructive and additive
/// (per `.context/usability.md`): it never hijacks right-click, dismisses
/// on a new selection or Escape, and sits out of the way at the
/// selection's end. Clicking opens the composer. Beside it, Add to Context
/// puts the selected text in the thread's basket.
export function SelectionCommentToolbar({
  rect,
  onAdd,
  onAddContext,
}: {
  rect: DOMRect;
  onAdd: () => void;
  /** Put the selection in the thread's context basket; omitted, no button. */
  onAddContext?: () => void;
}) {
  const left = Math.min(Math.max(8, rect.left), window.innerWidth - (onAddContext ? 300 : 140));
  const top = Math.max(8, rect.top - 38);
  // Use the down-event so we act before the selection collapses, and
  // keep it from bubbling to the document mouseup capture.
  const press = (run: () => void) => (e: { preventDefault(): void; stopPropagation(): void }) => {
    e.preventDefault();
    e.stopPropagation();
    run();
  };
  return (
    <div data-selection-comment-ui style={{ position: "fixed", left, top, zIndex: 1000, display: "inline-flex", gap: 6 }}>
      <button type="button" data-selection-comment-ui data-testid="selection-comment-button" onMouseDown={press(onAdd)} style={buttonStyle}>
        <MessageSquarePlus size={14} aria-hidden />
        Add comment
      </button>
      {onAddContext ? (
        <button type="button" data-selection-comment-ui data-testid="selection-context-button" onMouseDown={press(onAddContext)} style={buttonStyle}>
          <AtSign size={14} aria-hidden />
          Add to Context
        </button>
      ) : null}
    </div>
  );
}

const buttonStyle = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  padding: "5px 10px",
  fontFamily: "var(--font-ui)",
  fontSize: 12,
  color: "var(--text-primary)",
  background: "var(--surface-elevated)",
  border: "1px solid var(--border-strong)",
  borderRadius: 6,
  boxShadow: "0 4px 14px rgba(0,0,0,0.35)",
  cursor: "pointer",
} as const;
