/// Whether `event` is a plain PageUp / PageDown, which the terminal pane
/// turns into a page of xterm's own scrollback.
export function shouldHandleTerminalPageKey(event: {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
    return false;
  }
  return event.key === "PageUp" || event.key === "PageDown";
}
