/**
 * A column with `format: items`: its value is a list of items, drawn
 * inline separated by " · ". An item's `hint` is its hover text (and its
 * accessible description); its `uri` makes it a link to that page.
 */
import { Fragment, type CSSProperties } from "react";

import { RouteLink } from "../tabs/RouteLink.js";
import { parseUri } from "../uri/uri.js";
import type { Item } from "./vizModel.js";

export function ItemsCell({
  items,
  linkStyle,
}: {
  items: readonly Item[];
  /** How a linked item looks (the viz's link). */
  linkStyle: CSSProperties;
}) {
  return (
    <span data-testid="viz-items">
      {items.map((item, i) => (
        <Fragment key={i}>
          {i > 0 ? " · " : null}
          {item.uri && parseUri(item.uri) ? (
            <RouteLink to={item.uri} testId="viz-item" title={item.hint} style={linkStyle}>
              {item.text}
            </RouteLink>
          ) : (
            <span data-testid="viz-item" title={item.hint} aria-description={item.hint}>
              {item.text}
            </span>
          )}
        </Fragment>
      ))}
    </span>
  );
}
