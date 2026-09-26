import { useEffect, useRef } from "react";
import { loadSvgPanZoom } from "./mermaidRender.js";

export type LightboxContent =
  | { kind: "image"; src: string; alt?: string }
  | { kind: "svg"; html: string };

/**
 * Full-screen overlay used by `MarkdownView` to expand images and
 * mermaid diagrams. Closes on Escape, on backdrop click, or via the
 * top-right close affordance. For SVG content we re-mount a fresh
 * svg-pan-zoom instance inside the overlay so the user gets a large
 * interactive view; the inline pan-zoom on the page is independent.
 */
export function MediaLightbox({ content, onClose }: { content: LightboxContent | null; onClose(): void }) {
  const svgHostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!content) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [content, onClose]);

  useEffect(() => {
    if (!content || content.kind !== "svg") return;
    const host = svgHostRef.current;
    if (!host) return;
    host.innerHTML = content.html;
    const svg = host.querySelector<SVGSVGElement>("svg");
    if (!svg) return;
    svg.removeAttribute("style");
    svg.setAttribute("width", "100%");
    svg.setAttribute("height", "100%");
    let cancelled = false;
    let destroy: (() => void) | null = null;
    void (async () => {
      // Wait one frame for the flex layout to size the host before
      // svg-pan-zoom reads its rect — initialising at 0×0 makes
      // fit/center produce NaN transforms and the diagram stays
      // invisible.
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      if (cancelled) return;
      const svgPanZoom = await loadSvgPanZoom();
      if (cancelled) return;
      const instance = svgPanZoom(svg, {
        zoomEnabled: true,
        mouseWheelZoomEnabled: true,
        panEnabled: true,
        controlIconsEnabled: true,
        fit: true,
        center: true,
        minZoom: 0.2,
        maxZoom: 40,
        contain: false,
      });
      destroy = () => { try { instance.destroy(); } catch { /* ignore */ } };
    })();
    return () => {
      cancelled = true;
      destroy?.();
      host.innerHTML = "";
    };
  }, [content]);

  if (!content) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0, 0, 0, 0.82)",
        zIndex: 9999,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 32,
      }}
    >
      <button
        type="button"
        onClick={onClose}
        aria-label="Close"
        style={{
          position: "absolute",
          top: 16,
          right: 16,
          width: 36,
          height: 36,
          borderRadius: 18,
          background: "rgba(255, 255, 255, 0.12)",
          border: "1px solid rgba(255, 255, 255, 0.2)",
          color: "#fff",
          fontSize: 18,
          lineHeight: 1,
          cursor: "pointer",
          zIndex: 1,
        }}
      >
        ✕
      </button>
      {content.kind === "image" ? (
        <img
          src={content.src}
          alt={content.alt ?? ""}
          style={{
            maxWidth: "100%",
            maxHeight: "100%",
            objectFit: "contain",
            boxShadow: "0 8px 32px rgba(0, 0, 0, 0.5)",
            borderRadius: 6,
          }}
        />
      ) : (
        <div
          ref={svgHostRef}
          onClick={(e) => e.stopPropagation()}
          style={{
            // Explicit size — flexbox auto-sizing with svg-pan-zoom is
            // flaky; the diagram needs a real rect before init.
            width: "min(1400px, calc(100vw - 96px))",
            height: "calc(100vh - 96px)",
            background: "var(--surface-card)",
            borderRadius: 6,
            position: "relative",
            overflow: "hidden",
          }}
        />
      )}
    </div>
  );
}
