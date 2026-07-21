/**
 * Bare trend line — no axes, labels, or interaction. Lifted out of
 * `MetricsPage` when dashboard tiles needed the same mark (tsk142); the
 * 90×22 default keeps the metrics-list rows pixel-identical to before.
 *
 * `responsive` swaps the fixed width for a viewBox that scales to the
 * container (capped at `width`), which is what a dashboard tile wants.
 */
export function Sparkline({
  values,
  color,
  width = 90,
  height = 22,
  responsive = false,
}: {
  values: number[];
  color?: string;
  width?: number;
  height?: number;
  /** Scale to the container width (viewBox) instead of rendering at a fixed size. */
  responsive?: boolean;
}) {
  if (values.length < 2) return <span style={{ opacity: 0.35 }}>—</span>;
  const w = width;
  const h = height;
  const pad = 2;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pts = values
    .map((v, i) => {
      const x = pad + (i / (values.length - 1)) * (w - 2 * pad);
      const y = h - pad - ((v - min) / range) * (h - 2 * pad);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg
      width={responsive ? undefined : w}
      height={responsive ? undefined : h}
      viewBox={responsive ? `0 0 ${w} ${h}` : undefined}
      preserveAspectRatio={responsive ? "none" : undefined}
      style={responsive ? { display: "block", width: "100%", height } : { display: "block" }}
      aria-hidden
    >
      <polyline points={pts} fill="none" stroke={color ?? "var(--accent, #58a6ff)"} strokeWidth={1.5} />
    </svg>
  );
}
