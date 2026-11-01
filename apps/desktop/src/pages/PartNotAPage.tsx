/// A route to a part or a query: neither has a tab of its own. A part is
/// drawn inside the pages that host it, so the tab names them; a query is
/// rows other pages read. Said in its place, never an error.
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page } from "../tabs/Page.js";
import type { Extension, Page as PageFile } from "../tauri-bridge/generated/bindings.js";
import { hostsOf } from "./pageFiles.js";

export function PartNotAPage({ page, extensions }: { page: PageFile; extensions: readonly Extension[] }) {
  const hosts = page.kind === "part" ? hostsOf(extensions, page.id) : [];
  const text =
    page.kind === "query"
      ? `\`${page.id}\` is a query: rows other pages read (a choice's options, a form's choices). It has no tab of its own.`
      : hosts.length > 0
        ? `\`${page.id}\` is a part, drawn inside ${hosts.join(", ")}. It has no tab of its own.`
        : `\`${page.id}\` is a part, drawn only inside a host page. It has no tab of its own, and nothing hosts it yet.`;
  return (
    <Page testId="page-not-a-page" title={page.title}>
      <div style={{ padding: 16 }}>
        <EmptyState title={page.kind === "query" ? "A query, not a page" : "A part, not a page"} text={text} />
      </div>
    </Page>
  );
}
