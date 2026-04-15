import type { CommandId } from "./commands.js";

export function getCommandIdForShortcut(event: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}): CommandId | null {
  if (event.altKey || !(event.metaKey || event.ctrlKey)) {
    return null;
  }

  switch (event.key.toLowerCase()) {
    case "s":
      return "file.save";
    case "p":
      return "file.quickOpen";
    case "f":
      return "edit.find";
    default:
      return null;
  }
}
