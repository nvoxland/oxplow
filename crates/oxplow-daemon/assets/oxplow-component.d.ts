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
  streamId: string | null;
  threadId: string | null;
}

/** A lens spec a page shows (`showSpec`), as `oxplow.lens.keep` takes one. */
export interface OxplowSpec {
  title: string;
  query: string;
  /** `table`, `list`, `number`, `markdown`, `bar`, `line`, `treemap`, … */
  viz: string;
  /** A chart's columns: `x`, `y`, `series`, `label`, `size`, `group`. */
  chart?: Record<string, string | null>;
  description?: string;
}

/** An action drawn in the page's bar. */
/** How a sign-in the host ran ended. */
export type OxplowSignInResult = { state: "signed_in" } | { state: "failed"; error: string } | { state: "cancelled" };

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
  /** Open a form of the component's own: `schema` is a JSON Schema for its
   *  fields (a `writeOnly` string is masked), starting from `values`.
   *  Resolves with the values submitted, running nothing; rejects
   *  `CANCELLED` when the person closes it. A page only. */
  openForm(form: { title: string; schema: Record<string, unknown>; values?: Record<string, unknown> }): Promise<Record<string, unknown>>;
  /** Sign in for a provider instance's credential (one its provider
   *  declares with `oauth:`). oxplow opens the service's page in the
   *  person's browser, listens for the redirect on their machine and
   *  finishes the sign-in, then resolves how it ended. The component
   *  declares `oxplow.provider.begin_sign_in`, `complete_sign_in` and
   *  `cancel_sign_in` in `commands`. Rejects `INVALID` without the
   *  desktop app. A page only. */
  signIn(instance: string, name: string): Promise<OxplowSignInResult>;
  /** End that sign-in under way: its {@link signIn} resolves `cancelled`. A page only. */
  cancelSignIn(instance: string, name: string): Promise<null>;
  /** The page's title, in its tab and bar. A page only. */
  setTitle(title: string): Promise<null>;
  /** Have oxplow draw a lens spec under the page with its own components,
   *  live, with Save as Lens and Pin to Dashboard; null clears it.
   *  Resolves with the rows it drew (null when cleared). Rejects
   *  `INVALID` with the `field` it is about (`/spec/query`) for a spec
   *  oxplow refuses. A page only. While one shows, the frame sits at its
   *  content's height: leave `html` and `body` at `height: auto`. */
  showSpec(spec: OxplowSpec | null): Promise<OxplowRows | null>;
  /** The page's actions, drawn by oxplow in its bar. A page only. */
  setActions(actions: OxplowAction[]): Promise<null>;
  /** Hear each press of a page action, by id; returns how to stop. */
  onAction(listener: (id: string) => void): () => void;
  /** **View params** — its tab's address, as far as it is the
   *  component's: a page's own (`page:x/timeline?from=…`), a replacement's
   *  (the keys its target's contract lists, `v_replaceable.view`) or a
   *  lens page's (the lens's params). Null when its host gives no view (a
   *  dashboard tile, a slot). */
  readonly viewParams: Record<string, string> | null;
  /** Change its view params (null drops one): its tab's entry is replaced,
   *  so Back, a bookmark and a reopened tab keep them. A host with a view
   *  only; a key not its to set rejects `INVALID`. */
  setViewParams(params: Record<string, string | null>): Promise<null>;
  /** Hear its view params each time they change; returns how to stop. */
  onViewParams(listener: (params: Record<string, string>) => void): () => void;
  /** **History state** — this step of its tab's history (scroll, what's
   *  expanded, a draft): its own, apart from its host's. Null when its
   *  host gives no view. */
  readonly historyState: Record<string, unknown> | null;
  /** Merge `patch` into its history state. A host with a view only. */
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
