/// The kinds of ref the window opens in code of its own, instead of as a
/// page tab (`App.handleOpenPage` supplies one handler per kind). Every
/// other kind opens as a per-thread page tab. A closed list on purpose: it
/// only shrinks (`openHandlers.test.ts` pins it against
/// `openHandlers.golden.json`) — a kind leaves it when its page opens it, and
/// a new kind is a page, not a handler here.
import type { TabRef } from "./tabState.js";

export const OPEN_HANDLER_KINDS = ["oxplow/agent_session", "oxplow/symbol", "oxplow/file"] as const;

export type OpenHandlerKind = (typeof OPEN_HANDLER_KINDS)[number];

/** One handler per kind in [`OPEN_HANDLER_KINDS`]. */
export type OpenHandlers = Record<OpenHandlerKind, (ref: TabRef) => void>;

/** The handler for ref kind `kind`, when the window has one. */
export function openHandlerFor(handlers: OpenHandlers, kind: string | null | undefined): ((ref: TabRef) => void) | undefined {
  return (OPEN_HANDLER_KINDS as readonly string[]).includes(kind ?? "") ? handlers[kind as OpenHandlerKind] : undefined;
}
