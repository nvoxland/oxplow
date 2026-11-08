/// A route no enabled extension has a page file for: said in its place,
/// never an error.
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page } from "../tabs/Page.js";
import type { PagePayload } from "../tabs/pageRefs.js";

export function MissingPage({ page }: { page: PagePayload }) {
  const name = page.namespace ? `${page.namespace}/${page.page}` : page.page;
  return (
    <Page testId="page-missing" title={name}>
      <div style={{ padding: 16 }}>
        <EmptyState title="No such page" text={`No enabled extension declares the page \`${name}\`.`} />
      </div>
    </Page>
  );
}
