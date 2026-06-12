/// Gating for the floating "Add comment" toolbar over a fresh text
/// selection inside a comment-enabled RichTextField. The Tiptap surface
/// is deliberately carved out of the plain-DOM selection layer
/// (`useDomAnnotations` skips contenteditable), so the field shows its
/// own affordance — this predicate keeps the rules in one testable spot.
export interface SelectionToolbarInput {
  /// The field has a `comments` config (commentable target).
  commentsEnabled: boolean;
  /// The editor selection is collapsed.
  selectionEmpty: boolean;
  /// The new-comment composer is open for a pending selection.
  composerOpen: boolean;
  /// An existing comment-thread popover is open.
  popoverOpen: boolean;
}

export function selectionToolbarVisible(input: SelectionToolbarInput): boolean {
  return (
    input.commentsEnabled && !input.selectionEmpty && !input.composerOpen && !input.popoverOpen
  );
}
