/// A declared page that a lens draws (or, with `component:`, a custom
/// component: `CustomPageView`) (`pages:` with `lens:`), full-page,
/// starting with the page id's params (`?uri=` when it opens one of the
/// extension's refs). The page resolves from the extensions the app shows
/// (the main worktree's), so a restored tab (its id alone) opens too.
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page } from "../tabs/Page.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { ExtensionPage, Stream } from "../tauri-bridge/generated/bindings.js";
import { LensPage } from "./LensPage.js";
import { CustomPageView } from "./CustomPageView.js";
import { tabLabel } from "./renderers.js";

export function DeclaredPageView({
  page,
  declared,
  stream,
  threadId = null,
  onOpenPage,
}: {
  page: PagePayload;
  /// `undefined` while the extensions load.
  declared: ExtensionPage | null | undefined;
  stream: Stream | null;
  /// The thread the page is shown in, which a custom page is told.
  threadId?: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  if (declared === undefined) return null;
  if (declared !== null && declared.body.kind === "component") {
    return <CustomPageView page={page} declared={declared} stream={stream} threadId={threadId} onOpenPage={onOpenPage} />;
  }
  if (declared === null || declared.body.kind !== "lens") {
    const name = page.extension ? `${page.extension}/${page.page}` : page.page;
    return (
      <Page testId="page-missing" title={name} kind="page">
        <div style={{ padding: 16 }}>
          <EmptyState title="No such page" text={`No enabled extension declares the page \`${name}\`.`} />
        </div>
      </Page>
    );
  }
  return (
    <LensPage
      lensId={declared.body.lens}
      title={tabLabel(declared, page.params)}
      params={page.params}
      stream={stream}
      onOpenPage={onOpenPage}
    />
  );
}
