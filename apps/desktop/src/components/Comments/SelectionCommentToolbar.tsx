import { MessageSquarePlus } from "lucide-react";

/// A small floating "Add comment" button shown over a fresh text
/// selection on a commentable surface. Non-destructive and additive
/// (per `.context/usability.md`): it never hijacks right-click, dismisses
/// on a new selection or Escape, and sits out of the way at the
/// selection's end. Clicking opens the composer.
export function SelectionCommentToolbar({
  rect,
  onAdd,
}: {
  rect: DOMRect;
  onAdd: () => void;
}) {
  const left = Math.min(Math.max(8, rect.left), window.innerWidth - 140);
  const top = Math.max(8, rect.top - 38);
  return (
    <button
      type="button"
      data-selection-comment-ui
      data-testid="selection-comment-button"
      // Use the down-event so we act before the selection collapses, and
      // keep it from bubbling to the document mouseup capture.
      onMouseDown={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onAdd();
      }}
      style={{
        position: "fixed",
        left,
        top,
        zIndex: 1000,
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
      }}
    >
      <MessageSquarePlus size={14} aria-hidden />
      Add comment
    </button>
  );
}
