import { ResourcePage } from "./ResourcePage.js";
import type { TabRef } from "../tabs/tabState.js";
import { openExternalUrl } from "../api.js";
import type { CenterTab } from "../components/CenterTabs/CenterTabs.js";
import type { BuiltInPageName } from "./declaredPages.js";
import type { SlotNav } from "./appScope.js";
import { ExternalUrlPage } from "./ExternalUrlPage.js";

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
  // A web page: its tab's id is its URL.
  "external-url": ({ ref }) => {
    const externalUrl = ref.id;
    let label = externalUrl;
    try {
      const u = new URL(externalUrl);
      label = u.host + (u.pathname && u.pathname !== "/" ? u.pathname : "");
    } catch { /* keep raw */ }
    return {
      id: ref.id,
      label: label.length > 40 ? label.slice(0, 40) + "…" : label,
      closable: true,
      contextMenu: [
        {
          id: "external-url.open-in-browser",
          label: "Open in Browser",
          enabled: true,
          run: () => { void openExternalUrl(externalUrl); },
        },
        {
          id: "external-url.copy",
          label: "Copy URL",
          enabled: true,
          run: () => { void navigator.clipboard.writeText(externalUrl).catch(() => {}); },
        },
      ],
      render: () => (
        <ExternalUrlPage
          url={externalUrl}
          onOpenInBrowser={(u) => { void openExternalUrl(u); }}
        />
      ),
    };
  },
};
