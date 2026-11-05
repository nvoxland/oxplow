/** Whether the keys go to a field outside `region`: a text field, a select
 *  or an editor that holds the focus. Something that takes the focus by
 *  itself (a form that opens, a terminal that starts) asks first, so it
 *  doesn't pull it from a person who is typing elsewhere: the blur would
 *  also close an entry that cancels on blur. */
export function typingElsewhere(region: HTMLElement): boolean {
  const held = document.activeElement;
  if (!(held instanceof HTMLElement) || held === document.body || region.contains(held)) return false;
  return held.isContentEditable || held instanceof HTMLInputElement || held instanceof HTMLTextAreaElement || held instanceof HTMLSelectElement;
}
