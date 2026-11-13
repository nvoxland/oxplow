/**
 * An agent harness's logo, wherever a session is shown (a session's tab,
 * the New Session page's earlier sessions). The harness declares it
 * (`icon:` on its `implementations:` entry), and `HarnessListing.icon`
 * carries it as an SVG data URI; a harness that declares none gets the
 * generic robot.
 *
 * The declared SVG is drawn as every extension svg icon is (`iconFor`):
 * a mask in the text's color, its markup never injected.
 */
import { Bot } from "lucide-react";
import type { CSSProperties } from "react";

import { iconFor } from "../icons.js";
import { isIconName } from "../iconNames.js";

export interface AgentIconProps {
  /** The harness's declared logo (`HarnessListing.icon`). */
  icon?: string | null;
  size?: number;
  style?: CSSProperties;
}

export function AgentIcon({ icon, size = 14, style }: AgentIconProps) {
  const Declared = icon && !isIconName(icon) ? iconFor(icon) : null;
  if (Declared) {
    return (
      <span aria-hidden data-testid="agent-icon" data-icon="declared" style={{ display: "inline-flex", ...style }}>
        <Declared size={size} />
      </span>
    );
  }
  return (
    <span aria-hidden data-testid="agent-icon" data-icon="fallback" style={{ display: "inline-flex", ...style }}>
      <Bot size={size} />
    </span>
  );
}
