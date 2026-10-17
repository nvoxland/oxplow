import { Fragment, type HTMLAttributes, type Key, type ReactNode } from "react";

export interface DataColumn<R> {
  key: string;
  label: ReactNode;
  /** A fixed width (`60`, `"8rem"`); the columns without one share the
   *  rest. Any width makes the table lay out fixed, so columns line up. */
  width?: number | string;
  /** A number: right-aligned, in tabular figures. */
  numeric?: boolean;
  /** Cut a long cell to one line; `title` gives the whole text on hover. */
  truncate?: boolean;
  render(row: R): ReactNode;
  /** The cell's hover text. */
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
  /** A cell's own style beyond the column's (a row's indent). */
  cellStyle?(row: R, column: DataColumn<R>, columnIndex: number): React.CSSProperties | undefined;
  /** Draw the column headings (default). */
  header?: boolean;
  testId?: string;
  /** For a list driven from an input (`useListNav`'s `listProps`). */
  tableProps?: HTMLAttributes<HTMLTableElement>;
}

/** The one table: core pages and lenses draw rows through it. */
export function DataTable<R>({ columns, rows, groups, rowKey, rowProps, cellStyle, header = true, testId, tableProps }: DataTableProps<R>) {
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
              return (
                <tr key={rowKey(row, i)} {...rowProps?.(row, i)}>
                  {columns.map((c, j) => (
                    <td
                      key={c.key}
                      className={[c.numeric ? "ox-num" : "", c.truncate ? "ox-truncate" : ""].filter(Boolean).join(" ") || undefined}
                      title={c.title?.(row)}
                      style={cellStyle?.(row, c, j)}
                    >
                      {c.render(row)}
                    </td>
                  ))}
                </tr>
              );
            })}
          </Fragment>
        ))}
      </tbody>
    </table>
  );
}
