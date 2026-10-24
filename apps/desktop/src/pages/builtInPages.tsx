import { ResourcePage } from "./ResourcePage.js";
import type { TabRef } from "../tabs/tabState.js";
import type { CenterTab } from "../components/CenterTabs/CenterTabs.js";
import type { BuiltInPageName } from "./declaredPages.js";
import type { SlotNav } from "./appScope.js";

/** What a built-in page body gets: its tab ref and the ref's payload, and the slot's navigation. */
export interface BuiltInPageContext {
  ref: TabRef;
  /** The id's params (`pagePayload(ref).params`). */
  params: Record<string, string>;
  nav: SlotNav;
}

export type BuiltInPage = (ctx: BuiltInPageContext) => CenterTab | null;

export const builtInPages: Record<BuiltInPageName, BuiltInPage> = {
  // What a URI no page renders opens on.
  resource: ({ ref }) => ({
    id: ref.id,
    label: "",
    closable: true,
    render: () => <ResourcePage uri={ref.id} />,
  }),
};
