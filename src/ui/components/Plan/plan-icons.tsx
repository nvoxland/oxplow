import type { CSSProperties } from "react";
import type { WorkItemPriority } from "../../api.js";

/**
 * Three-bar priority glyph mirroring Linear's scannable column. Urgent /
 * High / Medium / Low each render the same three-bar footprint so rows
 * don't shift width when priority changes — only the number of filled bars
 * and the fill colour vary.
 *
 *   urgent: ▬▬▬  filled, --priority-urgent
 *   high:   ▬▬·  two bars filled, --priority-high
 *   medium: ▬··  one bar filled, --priority-medium
 *   low:    ···  three empty bars, --priority-low
 */
export function PriorityIcon({ priority, size = 10 }: { priority: WorkItemPriority; size?: number }) {
  const filled = priority === "urgent" ? 3 : priority === "high" ? 2 : priority === "medium" ? 1 : 0;
  const color = priorityColorVar(priority);
  const barWidth = Math.max(2, Math.round(size * 0.2));
  const barGap = Math.max(1, Math.round(size * 0.15));
  const totalWidth = barWidth * 3 + barGap * 2;
  const heights = [Math.round(size * 0.5), Math.round(size * 0.75), size];

  return (
    <span
      aria-label={`Priority: ${priority}`}
      style={iconFrameStyle(size, totalWidth)}
    >
      {heights.map((h, i) => {
        const active = i < filled;
        return (
          <span
            key={i}
            style={{
              width: barWidth,
              height: h,
              borderRadius: 1,
              background: active ? color : "transparent",
              border: active ? "none" : `1px solid ${color}`,
              opacity: active ? 1 : 0.45,
              boxSizing: "border-box",
            }}
          />
        );
      })}
    </span>
  );
}

function priorityColorVar(priority: WorkItemPriority): string {
  switch (priority) {
    case "urgent": return "var(--priority-urgent)";
    case "high": return "var(--priority-high)";
    case "medium": return "var(--priority-medium)";
    case "low": return "var(--priority-low)";
  }
}

function iconFrameStyle(size: number, totalWidth: number): CSSProperties {
  return {
    display: "inline-flex",
    alignItems: "flex-end",
    gap: Math.max(1, Math.round(size * 0.15)),
    width: totalWidth,
    height: size,
    flexShrink: 0,
  };
}
