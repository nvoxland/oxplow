// Types for oxplow's component client library (`oxplow-component.js`),
// which defines the global `oxplow`. Copy this beside your bundle's
// sources, or reference it, to type-check them.

/** A value in a result row or a param. */
export type OxplowCell = string | number | boolean | null;

/** One run of a lens: what it is, what it was asked, what it answered. */
export interface OxplowRun {
  lens: { id: string; title: string; [key: string]: unknown };
  params: Record<string, OxplowCell>;
  result: { columns: string[]; rows: OxplowCell[][]; truncated: boolean };
}

/** Why a request failed (`DENIED`, `INVALID`, `CANCELLED`, `FAILED`, …). */
export interface OxplowError {
  code: string;
  message: string;
  /** For `INVALID` input, the field it names (a JSON pointer, `/title`). */
  field?: string;
}

/** A result of `querySql`. */
export interface OxplowRows {
  columns: string[];
  rows: OxplowCell[][];
  truncated: boolean;
}

/** The page a component is, when it is one (`pages: [{ component }]`). */
export interface OxplowPage {
  /** The page's params, from its id (`page:x/timeline?from=…`). */
  params: Record<string, string>;
  streamId: string | null;
  threadId: string | null;
}

/** An action drawn in the page's bar. */
export interface OxplowAction {
  id: string;
  label: string;
  primary?: boolean;
  disabled?: boolean;
}

export interface OxplowComponent {
  /** The lens's latest run; null for a component that is a page. */
  readonly run: OxplowRun | null;
  /** The page it is, or null when a lens shows it. */
  readonly page: OxplowPage | null;
  /** The lens's `custom.props`, or null. */
  readonly props: unknown;
  /** The theme's CSS variables, by name (`--text-primary`). */
  readonly tokens: Record<string, string>;
  /** The protocol the host speaks. */
  readonly protocol: number;
  /** Hear each re-run of the lens; returns how to stop. */
  onUpdate(listener: (run: OxplowRun) => void): () => void;
  /** Run a lens this component declares in `assets`. Rejects with an {@link OxplowError}. */
  query(asset: string, params?: Record<string, OxplowCell>): Promise<OxplowRun>;
  /** Run a command this component declares in `commands`, as the person
   *  looking at it; one that asks is confirmed by them, in oxplow.
   *  Rejects with an {@link OxplowError} (`CANCELLED` when they decline;
   *  `DENIED` when a policy refuses it, or the bundle it was loaded at is
   *  no longer the deployed one). */
  invoke(command: string, input?: unknown): Promise<unknown>;
  /** Open one of oxplow's pages, in a new tab with `{ newTab: true }`.
   *  Rejects with an {@link OxplowError}. */
  navigate(ref: string, options?: { newTab?: boolean }): Promise<null>;
  /** Run read-only SQL over oxplow's models (`?1` parameters). Needs
   *  `sql: read` on the component and `permissions.sql: read` on its
   *  extension. Rejects with an {@link OxplowError}. */
  querySql(sql: string, params?: OxplowCell[]): Promise<OxplowRows>;
  /** Open the form of a command this component declares, in oxplow,
   *  starting from `input`: resolves with its result once submitted,
   *  rejects `CANCELLED` when the person closes it. A page only. */
  openForm(command: string, input?: unknown): Promise<unknown>;
  /** The page's title, in its tab and bar. A page only. */
  setTitle(title: string): Promise<null>;
  /** The page's actions, drawn by oxplow in its bar. A page only. */
  setActions(actions: OxplowAction[]): Promise<null>;
  /** Hear each press of a page action, by id; returns how to stop. */
  onAction(listener: (id: string) => void): () => void;
  /** **View params** — its tab's address: what the page shows. A page
   *  only; null when a lens shows it. */
  readonly viewParams: Record<string, string> | null;
  /** Change its view params (null drops one): its tab's entry is replaced,
   *  so Back, a bookmark and a reopened tab keep them. A page only. */
  setViewParams(params: Record<string, string | null>): Promise<null>;
  /** Hear its view params each time they change; returns how to stop. */
  onViewParams(listener: (params: Record<string, string>) => void): () => void;
  /** **History state** — this step of its tab's history (scroll, what's
   *  expanded, a draft). A page only; null when a lens shows it. */
  readonly historyState: Record<string, unknown> | null;
  /** Merge `patch` into its history state. A page only. */
  setHistoryState(patch: Record<string, unknown>): Promise<null>;
  /** Hear its history state each time it changes; returns how to stop. */
  onHistoryState(listener: (state: Record<string, unknown>) => void): () => void;
  /** **Page memory** — the component's own, for the person, in every tab
   *  of it. Values are JSON: 64 KB each, 1 MB a component. */
  readonly pageMemory: {
    get(key: string): Promise<unknown>;
    set(key: string, value: unknown): Promise<null>;
  };
  /** Set each theme token, and the host's `color-scheme`, on the
   *  document's root, where the kit's stylesheet
   *  (`/component-lib/oxplow-kit.css`) reads them. Without the
   *  `color-scheme` the browser paints the frame opaque white. */
  applyTheme(doc?: Document): void;
}

declare global {
  const oxplow: {
    /** The protocol this library speaks. */
    readonly PROTOCOL: number;
    /** Wait for oxplow's `init` and answer `ready`. Rejects when none
     *  comes within `timeoutMs` (10 s) or the host speaks another protocol. */
    connect(options?: { timeoutMs?: number; target?: EventTarget }): Promise<OxplowComponent>;
  };
}
