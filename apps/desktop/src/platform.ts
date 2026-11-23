/// Which OS the window runs on, read from the navigator: it names OS
/// things the person sees (the file manager in Open In ▸, the shortcut
/// modifiers).

export type Platform = "mac" | "windows" | "linux";

/** The platform a navigator `platform` (or user agent) string names. Pure. */
export function platformFrom(text: string): Platform {
  if (/Mac|iPhone|iPad|iPod/.test(text)) return "mac";
  if (/Win/.test(text)) return "windows";
  return "linux";
}

/** This window's platform. */
export function currentPlatform(): Platform {
  return typeof navigator === "undefined" ? "linux" : platformFrom(navigator.platform || navigator.userAgent || "");
}

/** The Open In ▸ item that shows a path in the OS file manager. */
export function revealLabel(platform: Platform): string {
  switch (platform) {
    case "mac":
      return "Reveal in Finder";
    case "windows":
      return "Show in Explorer";
    case "linux":
      return "Show in File Manager";
  }
}
