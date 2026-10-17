/**
 * An agent harness's logo, wherever a session is shown (a session's tab,
 * the New Session page's earlier sessions). The harness declares it
 * (`icon:` on its `implementations:` entry), and `HarnessListing.icon`
 * carries it as an SVG data URI; a harness that declares none gets the
 * generic robot.
 *
 * The declared SVG is an extension's, so it is never injected as markup:
 * it is the mask of an empty element (an image load, where no script in
 * it runs), colored by the text around it. Only an SVG data URI is drawn
 * (anything else falls back), so nothing can leave the CSS `url()`.
 */
import { Bot } from "lucide-react";
import type { CSSProperties } from "react";

/** What a declared icon looks like on the wire: base64 only, so the URI
 *  holds no quote or parenthesis. */
const SVG_DATA_URI = /^data:image\/svg\+xml;base64,[A-Za-z0-9+/=]+$/;

export interface AgentIconProps {
  /** The harness's declared logo (`HarnessListing.icon`). */
  icon?: string | null;
  size?: number;
  style?: CSSProperties;
}

export function AgentIcon({ icon, size = 14, style }: AgentIconProps) {
  if (icon && SVG_DATA_URI.test(icon)) {
    const image = `url("${icon}")`;
    return (
      <span
        aria-hidden
        data-testid="agent-icon"
        data-icon="declared"
        style={{
          display: "inline-block",
          width: size,
          height: size,
          backgroundColor: "currentColor",
          WebkitMaskImage: image,
          maskImage: image,
          WebkitMaskSize: "contain",
          maskSize: "contain",
          WebkitMaskRepeat: "no-repeat",
          maskRepeat: "no-repeat",
          WebkitMaskPosition: "center",
          maskPosition: "center",
          ...style,
        }}
      />
    );
  }
  return (
    <span aria-hidden data-testid="agent-icon" data-icon="fallback" style={{ display: "inline-flex", ...style }}>
      <Bot size={size} />
    </span>
  );
}
