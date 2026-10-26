import { Fragment, type HTMLAttributes, type Key, type ReactNode } from "react";

export interface DataColumn<R> {
  key: string;
  label: ReactNode;
  /** A fixed width (`60`, `"8rem"`); the columns without one share the
   *  rest. Any width makes the table lay out fixed, so columns line up. */
  width?: number | string;
  /** A number: right-aligned, in tabular figures. */
  numeric?: boolean;
  /** Cut a long cell to one line. On by default for a column with a
   *  `width` (`false` lets it wrap); a cut cell shows its whole text on
   *  hover. */
  truncate?: boolean;
  render(row: R): ReactNode;
  /** The cell's hover text; else a cut cell's own text. */
  title?(row: R): string | undefined;
}

export interface DataGroup<R> {
  key: string;
  /** Drawn in a full-width row above the group's rows. */
  heading: ReactNode;
  rows: readonly R[];
}

/** What a row element takes: handlers, its test id, its state. */
export type RowProps = HTMLAttributes<HTMLTableRowElement> & { [data: `data-${string}`]: string | undefined };

export interface DataTableProps<R> {
  columns: readonly DataColumn<R>[];
  /** The rows, or `groups` of them under headings. */
  rows?: readonly R[];
  groups?: readonly DataGroup<R>[];
  rowKey(row: R, index: number): Key;
  /** Per row: handlers, test id, `className` (`ox-clickable`,
   *  `ox-selected`, `ox-inert`), aria state. */
  rowProps?(row: R, index: number): RowProps;
  /** Drawn in a full-width row under the row, when it returns something
   *  (a row opened into more). */
  rowDetail?(row: R, index: number): ReactNode;
  /** A cell's own style beyond the column's (a row's indent). */
  cellStyle?(row: R, column: DataColumn<R>, columnIndex: number): React.CSSProperties | undefined;
  /** Draw the column headings (default). */
  header?: boolean;
  testId?: string;
  /** For a list driven from an input (`useListNav`'s `listProps`). */
  tableProps?: HTMLAttributes<HTMLTableElement>;
}

/** Whether a column's cells are cut to one line. */
function cuts(c: Pick<DataColumn<unknown>, "truncate" | "width">): boolean {
  return c.truncate ?? c.width !== undefined;
}

/** A cut cell's hover: its whole text, only when the cut hides some. */
function showWhole(e: React.MouseEvent<HTMLTableCellElement>) {
  const td = e.currentTarget;
  if (td.scrollWidth > td.clientWidth) td.title = td.textContent ?? "";
  else td.removeAttribute("title");
}

/** The one table: core pages and viz nodes draw rows through it. */
export function DataTable<R>({ columns, rows, groups, rowKey, rowProps, rowDetail, cellStyle, header = true, testId, tableProps }: DataTableProps<R>) {
  const fixed = columns.some((c) => c.width !== undefined);
  const parts: readonly { key: string; heading: ReactNode | null; rows: readonly R[] }[] =
    groups ?? [{ key: "", heading: null, rows: rows ?? [] }];
  let index = 0;
  return (
    <table className={fixed ? "ox-table ox-fixed" : "ox-table"} data-testid={testId} {...tableProps}>
      {fixed ? (
        <colgroup>
          {columns.map((c) => (
            <col key={c.key} style={c.width === undefined ? undefined : { width: typeof c.width === "number" ? `${c.width}px` : c.width }} />
          ))}
        </colgroup>
      ) : null}
      {header ? (
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c.key} className={c.numeric ? "ox-num" : undefined}>
                {c.label}
              </th>
            ))}
          </tr>
        </thead>
      ) : null}
      <tbody>
        {parts.map((part) => (
          <Fragment key={part.key}>
            {part.heading !== null ? (
              <tr>
                <td className="ox-group" colSpan={Math.max(columns.length, 1)}>
                  {part.heading}
                </td>
              </tr>
            ) : null}
            {part.rows.map((row) => {
              const i = index++;
              const detail = rowDetail?.(row, i);
              return (
                <Fragment key={rowKey(row, i)}>
                <tr {...rowProps?.(row, i)}>
                  {columns.map((c, j) => {
                    const cut = cuts(c);
                    const title = c.title?.(row);
                    return (
                      <td
                        key={c.key}
                        className={[c.numeric ? "ox-num" : "", cut ? "ox-truncate" : ""].filter(Boolean).join(" ") || undefined}
                        title={title}
                        onMouseEnter={cut && title === undefined ? showWhole : undefined}
                        style={cellStyle?.(row, c, j)}
                      >
                        {c.render(row)}
                      </td>
                    );
                  })}
                </tr>
                {detail ? (
                  <tr>
                    <td colSpan={Math.max(columns.length, 1)}>{detail}</td>
                  </tr>
                ) : null}
                </Fragment>
              );
            })}
          </Fragment>
        ))}
      </tbody>
    </table>
  );
}
