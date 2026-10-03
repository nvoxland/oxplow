/// An extension's page (`page:ext.<extension>.<page>`, P6.G2): the lens
/// its manifest's `pages:` entry names, full-page, starting with the page
/// id's params (`?ref=` when it opens one of the extension's refs, P8.D7).
/// The page resolves from the stream's extensions, so a restored tab (its
/// id alone) opens too.
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useExtensions } from "../extensionsStore.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { ExtensionPage, Stream } from "../tauri-bridge/generated/bindings.js";
import { LensPage } from "./LensPage.js";

export function ExtensionPageView({
  extension,
  page,
  params,
  stream,
  onOpenPage,
}: {
  extension: string;
  page: string;
  params?: Record<string, string>;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const exts = useExtensions(stream?.id ?? null);
  const found: ExtensionPage | null | undefined =
    exts === null ? undefined : (exts.find((e) => e.name === extension && e.enabled)?.pages.find((p) => p.id === page) ?? null);

  if (found === undefined) return null;
  if (found === null) {
    return (
      <Page testId="page-ext" title={`${extension} — ${page}`} kind="ext-page">
        <div style={{ padding: 16 }}>
          <EmptyState
            title="No such page"
            text={`The extension \`${extension}\` has no page \`${page}\` (or it's disabled).`}
          />
        </div>
      </Page>
    );
  }
  return (
    <LensPage lensId={found.lens} title={found.title} initialParams={params} stream={stream} onOpenPage={onOpenPage} />
  );
}
