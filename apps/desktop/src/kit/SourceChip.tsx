import { Bot, Box, FileText, User } from "lucide-react";

import { RouteLink } from "../tabs/RouteLink.js";
import { type Source } from "../viz/vizModel.js";

export { sourceValue, type Source } from "../viz/vizModel.js";

/** The chip that says who caused an alert, the same on every Alerts row,
 *  so "from an agent" looks one way wherever it shows. */
const ICON = { agent: Bot, you: User, extension: Box, page: FileText } as const;
const HOVER = { agent: "An agent in ", you: "", extension: "The extension ", page: "A page acting for an agent: " } as const;

export function SourceChip({ source }: { source: Source }) {
  const Icon = ICON[source.kind];
  const title = `${HOVER[source.kind]}${source.label}`;
  const body = (
    <>
      <Icon size={12} aria-hidden />
      <span>{source.label}</span>
    </>
  );
  return (
    <span data-testid="source-chip" data-kind={source.kind} className="ox-badge" title={title} style={chipStyle}>
      {source.uri ? (
        <RouteLink to={source.uri} title={title} style={linkStyle}>
          {body}
        </RouteLink>
      ) : (
        body
      )}
    </span>
  );
}

const chipStyle = { display: "inline-flex", alignItems: "center", gap: 4, color: "var(--text-secondary)" } as const;
const linkStyle = { display: "inline-flex", alignItems: "center", gap: 4, color: "inherit", background: "none", border: 0, padding: 0, font: "inherit", cursor: "pointer" } as const;
