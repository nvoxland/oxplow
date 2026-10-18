/// A declared page that a lens draws (`pages:` with `lens:`), full-page,
/// starting with the page id's params (`?ref=` when it opens one of the
/// extension's refs). The page resolves from the extensions the app shows
/// (the main worktree's), so a restored tab (its id alone) opens too.
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page } from "../tabs/Page.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { ExtensionPage, Stream } from "../tauri-bridge/generated/bindings.js";
import { LensPage } from "./LensPage.js";

export function DeclaredPageView({
  page,
  declared,
  stream,
  onOpenPage,
}: {
  page: PagePayload;
  /// `undefined` while the extensions load.
  declared: ExtensionPage | null | undefined;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  if (declared === undefined) return null;
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
    <LensPage lensId={declared.body.lens} title={declared.title} initialParams={page.params} stream={stream} onOpenPage={onOpenPage} />
  );
}
