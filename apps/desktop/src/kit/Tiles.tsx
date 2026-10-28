import type { HTMLAttributes, ReactNode } from "react";

/** How much of a dashboard a tile takes: one cell, two across, two down,
 *  or the whole row. */
export type TileSize = "small" | "wide" | "tall" | "full";

/** A dashboard's grid: tiles fill it in rows, as many across as fit. */
export function TileGrid({ testId, children, ...rest }: HTMLAttributes<HTMLDivElement> & { testId?: string }) {
  return (
    <div className="ox-tiles" data-testid={testId} {...rest}>
      {children}
    </div>
  );
}

/** One place in the grid, spanning by `size`; what's in it is the page's
 *  (a `Tile`, a heading band). It takes the page's drag handlers. */
export function TileSlot({ size = "small", className, children, ...rest }: HTMLAttributes<HTMLDivElement> & { size?: TileSize }) {
  const span = size === "small" ? "" : ` ox-tile-${size}`;
  return (
    <div className={`ox-tile-slot${span}${className ? ` ${className}` : ""}`} {...rest}>
      {children}
    </div>
  );
}

/** A tile: a titled card. `title` may be a link the page builds; `hideTitle`
 *  keeps it for screen readers and leaves it off the card. */
export function Tile({ title, hideTitle = false, children, ...rest }: Omit<HTMLAttributes<HTMLElement>, "title"> & { title?: ReactNode; hideTitle?: boolean }) {
  return (
    <section className="ox-tile" {...rest}>
      {title ? <div className={hideTitle ? "ox-tile-title ox-sr-only" : "ox-tile-title"}>{title}</div> : null}
      <div className="ox-tile-body">{children}</div>
    </section>
  );
}
