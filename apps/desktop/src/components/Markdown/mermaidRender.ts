/**
 * Shared mermaid rendering pipeline. Used by `MarkdownView` (read-only
 * markdown bodies) and by the rich-text editor's `MermaidBlock` node
 * view, so both render diagrams identically — same pan-zoom toolbar,
 * same lightbox hook, same lazy mermaid initialization.
 *
 * The render is split into a single `renderMermaidInto(host, source)`
 * call so callers can place the rendered SVG wherever they need it
 * (sibling node in MarkdownView, child of a Tiptap NodeView in the
 * editor) without re-implementing the lazy mermaid bootstrap or the
 * stray-node sweep.
 */

let mermaidPromise: Promise<typeof import("mermaid").default> | null = null;
export function loadMermaid() {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((mod) => {
      mod.default.initialize({
        startOnLoad: false,
        theme: "dark",
        securityLevel: "loose",
        // Mermaid 12 made ELK the default layout; pin dagre so existing
        // wiki diagrams keep the layout they were written against.
        layout: "dagre",
      });
      return mod.default;
    });
  }
  return mermaidPromise;
}

type SvgPanZoomFn = (typeof import("svg-pan-zoom"));
let svgPanZoomPromise: Promise<SvgPanZoomFn> | null = null;
export function loadSvgPanZoom() {
  if (!svgPanZoomPromise) {
    svgPanZoomPromise = import("svg-pan-zoom").then((mod) => {
      const m = mod as unknown as { default?: SvgPanZoomFn };
      return (m.default ?? (mod as unknown as SvgPanZoomFn));
    });
  }
  return svgPanZoomPromise;
}

function waitForVisible(host: HTMLElement): Promise<void> {
  if (host.offsetWidth > 0 && host.offsetHeight > 0) return Promise.resolve();
  return new Promise((resolve) => {
    const obs = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting && host.offsetWidth > 0 && host.offsetHeight > 0) {
          obs.disconnect();
          resolve();
          return;
        }
      }
    });
    obs.observe(host);
  });
}

export async function attachPanZoom(
  host: HTMLElement,
  svgHtml: string,
  onExpand?: (svgHtml: string) => void,
): Promise<(() => void) | null> {
  const svg = host.querySelector<SVGSVGElement>("svg");
  if (!svg) return null;
  // svg-pan-zoom requires the SVG to have a width/height set.
  svg.removeAttribute("style");
  svg.setAttribute("width", "100%");
  svg.setAttribute("height", "100%");
  host.style.position = "relative";
  host.style.height = "480px";
  host.style.maxHeight = "70vh";
  host.style.border = "1px solid var(--border-subtle)";
  host.style.borderRadius = "6px";
  host.style.overflow = "hidden";
  // Block until the host has real dimensions — see waitForVisible
  // comment for the b775f12 hidden-tab interaction.
  await waitForVisible(host);
  const svgPanZoom = await loadSvgPanZoom();
  const instance = svgPanZoom(svg, {
    zoomEnabled: true,
    // Mouse-wheel zoom is hostile inside a scrollable wiki page —
    // users expect the wheel to scroll the article, not silently
    // resize the diagram. The +/− toolbar buttons drive zoom.
    mouseWheelZoomEnabled: false,
    panEnabled: true,
    controlIconsEnabled: false,
    fit: true,
    center: true,
    minZoom: 0.2,
    maxZoom: 20,
    contain: false,
  });
  const toolbar = document.createElement("div");
  toolbar.className = "mermaid-pz-toolbar";
  toolbar.style.cssText = [
    "position: absolute",
    "top: 6px",
    "right: 6px",
    "display: flex",
    "gap: 2px",
    "background: var(--surface-card)",
    "border: 1px solid var(--border-subtle)",
    "border-radius: 4px",
    "padding: 2px",
    "font-size: 12px",
    "z-index: 1",
  ].join(";");
  const makeBtn = (label: string, title: string, onClick: () => void) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = label;
    btn.title = title;
    btn.style.cssText = [
      "background: transparent",
      "border: none",
      "color: var(--text-primary)",
      "cursor: pointer",
      "padding: 2px 6px",
      "font-size: 12px",
      "line-height: 1",
      "min-width: 20px",
    ].join(";");
    btn.addEventListener("click", (e) => { e.preventDefault(); onClick(); });
    return btn;
  };
  toolbar.appendChild(makeBtn("−", "Zoom out", () => instance.zoomOut()));
  toolbar.appendChild(makeBtn("+", "Zoom in", () => instance.zoomIn()));
  toolbar.appendChild(makeBtn("Reset", "Reset view", () => { instance.resetZoom(); instance.center(); instance.fit(); }));
  if (onExpand) {
    toolbar.appendChild(makeBtn("⛶", "Open in lightbox", () => onExpand(svgHtml)));
  }
  host.appendChild(toolbar);
  return () => {
    try { instance.destroy(); } catch { /* ignore */ }
    toolbar.remove();
  };
}

/**
 * Render a mermaid diagram into `host`. Replaces host contents with
 * the SVG (or an inline error message). Returns a teardown that
 * disposes the pan-zoom instance + toolbar.
 *
 * On parse failure, mermaid v11 sometimes appends temp containers to
 * `document.body`; we sweep both shapes after every attempt so the
 * default "Syntax error in text" bomb doesn't linger.
 */
export async function renderMermaidInto(
  host: HTMLElement,
  source: string,
  idSeed: string | number,
  onExpand?: (svgHtml: string) => void,
): Promise<(() => void) | null> {
  const id = `mermaid-${idSeed}`;
  const sweepStray = () => {
    for (const stray of document.querySelectorAll(
      `#${CSS.escape(id)}, #d${CSS.escape(id)}`,
    )) {
      if (stray.closest(".mermaid-rendered")) continue;
      if (host.contains(stray)) continue;
      stray.remove();
    }
  };
  try {
    const mermaid = await loadMermaid();
    await mermaid.parse(source);
    const { svg } = await mermaid.render(id, source);
    host.innerHTML = "";
    const inner = document.createElement("div");
    inner.className = "mermaid-rendered";
    inner.innerHTML = svg;
    host.appendChild(inner);
    sweepStray();
    return attachPanZoom(inner, svg, onExpand ? () => onExpand(svg) : undefined);
  } catch (error) {
    sweepStray();
    host.innerHTML = "";
    const err = document.createElement("div");
    err.style.color = "var(--severity-critical)";
    err.style.fontSize = "12px";
    err.style.padding = "8px";
    err.textContent = `Mermaid parse error: ${String(error)}`;
    host.appendChild(err);
    return null;
  }
}
