import { ResourcePage } from "./ResourcePage.js";
import type { TabRef } from "../tabs/tabState.js";
import { openExternalUrl } from "../api.js";
import type { CenterTab } from "../components/CenterTabs/CenterTabs.js";
import type { BuiltInPageName } from "./declaredPages.js";
import type { AppScope, SlotNav } from "./appScope.js";
import { TerminalPage } from "./TerminalPage.js";
import { ExternalUrlPage } from "./ExternalUrlPage.js";

/** What a built-in page body gets: its tab ref and the ref's payload, the slot's navigation, and core's scope. */
export interface BuiltInPageContext {
  ref: TabRef;
  /** The id's params (`pagePayload(ref).params`). */
  params: Record<string, string>;
  nav: SlotNav;
  scope: AppScope;
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
  terminal: ({ ref, scope }) => {
    return {
      id: ref.id,
      label: "Terminal",
      closable: true,
      render: () => (
        <TerminalPage
          stream={scope.stream}
          visible={scope.effectiveCenterActive === ref.id}
          onOpenFile={(absPath, line, column) => {
            if (!scope.stream) return;
            const wt = scope.stream.worktree_path.endsWith("/")
              ? scope.stream.worktree_path.slice(0, -1)
              : scope.stream.worktree_path;
            const rel = absPath.startsWith(wt + "/")
              ? absPath.slice(wt.length + 1)
              : absPath;
            if (typeof line === "number" && line > 0) {
              void scope.handleNavigateToLocation({ path: rel, line, column: column ?? 1 });
            } else {
              void scope.handleOpenFile(rel);
            }
          }}
        />
      ),
    };
  },
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
