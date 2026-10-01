import type { CSSProperties } from "react";

export type RefBadgeTone = "branch" | "current" | "tag" | "sha" | "wiki" | "label";

const TONE_STYLES: Record<RefBadgeTone, CSSProperties> = {
  branch: { borderColor: "#4a9eff", color: "#4a9eff" },
  current: { borderColor: "#86efac", color: "#86efac", fontWeight: 600 },
  tag: { borderColor: "#fcd34d", color: "#fcd34d" },
  sha: { borderColor: "#4a9eff", color: "#4a9eff" },
  wiki: { borderColor: "#c084fc", color: "#c084fc" },
  // An extension's decoration (P6b.C5); its own color, when it has one.
  label: { borderColor: "var(--border-subtle)", color: "var(--text-secondary)" },
};

/**
 * Pill badge for a git ref or short sha. Shared between the git
 * history graph rows and the Local History dashboard's snapshot rows
 * so the visual language for "what commit is this on" is consistent.
 */
export function RefBadge({ label, tone, color, title }: { label: string; tone: RefBadgeTone; color?: string; title?: string }) {
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        border: "1px solid",
        borderRadius: 999,
        padding: "0 6px",
        fontSize: 10,
        lineHeight: "14px",
        flexShrink: 0,
        fontFamily: tone === "sha" ? "var(--mono, monospace)" : undefined,
        ...TONE_STYLES[tone],
        ...(color ? { borderColor: color, color } : null),
      }}
      title={
        title ??
        (tone === "tag"
          ? `tag: ${label}`
          : tone === "sha"
          ? `commit ${label}`
          : tone === "wiki"
          ? `wiki page: ${label}`
          : label)
      }
    >
      {tone === "tag" ? "🏷 " : tone === "wiki" ? "📝 " : ""}
      {label}
    </span>
  );
}
