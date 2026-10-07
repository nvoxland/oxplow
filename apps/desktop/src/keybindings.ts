import type { CommandId } from "./commands.js";

/** A command bus command a shortcut runs through its offer
 *  (`commandOffers`): New Task opens its form. */
export type BusShortcut = "oxplow.work_item.create";

export function getCommandIdForShortcut(event: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}): CommandId | BusShortcut | null {
  if (event.altKey || !(event.metaKey || event.ctrlKey)) {
    return null;
  }

  const key = event.key.toLowerCase();
  if (event.shiftKey) {
    if (key === "n") return "oxplow.work_item.create";
    return null;
  }

  switch (key) {
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
