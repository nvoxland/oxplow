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
}

export interface OxplowComponent {
  /** The lens's latest run. */
  readonly run: OxplowRun;
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
   *  `DENIED` until a person approves the component in Settings → Data →
   *  Programs). */
  invoke(command: string, input?: unknown): Promise<unknown>;
  /** Open one of oxplow's pages. Rejects with an {@link OxplowError}. */
  navigate(ref: string): Promise<null>;
  /** Set each theme token on the document's root, where the kit's
   *  stylesheet (`/component-lib/oxplow-kit.css`) reads them. */
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
