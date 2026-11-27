import { ResourcePage } from "./ResourcePage.js";
import type { TabRef } from "../tabs/tabState.js";
import type { WindowTab } from "../window/windowTab.js";
import type { SlotNav } from "./slotNav.js";

/** What a built-in page body gets: its tab ref and the ref's payload, and the slot's navigation. */
export interface BuiltInPageContext {
  ref: TabRef;
  /** The id's params (`pagePayload(ref).params`). */
  params: Record<string, string>;
  nav: SlotNav;
}

export type BuiltInPage = (ctx: BuiltInPageContext) => WindowTab | null;

/** Core's one built-in page: what a URI no page file renders draws. */
export const builtInPages: Record<"resource", BuiltInPage> = {
  // What a URI no page renders opens on.
  resource: ({ ref }) => ({
    id: ref.id,
    label: "",
    closable: true,
    render: () => <ResourcePage uri={ref.id} />,
  }),
};
