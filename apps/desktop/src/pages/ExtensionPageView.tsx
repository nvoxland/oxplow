/// An extension's page (`page:ext.<extension>.<page>`, P6.G2): the lens
/// its manifest's `pages:` entry names, full-page. The page resolves from
/// the stream's extensions, so a restored tab (its id alone) opens too.
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useExtensions } from "../extensionsStore.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { ExtensionPage, Stream } from "../tauri-bridge/generated/bindings.js";
import { LensPage } from "./LensPage.js";

export function ExtensionPageView({
  extension,
  page,
  stream,
  onOpenPage,
}: {
  extension: string;
  page: string;
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
  return <LensPage lensId={found.lens} title={found.title} stream={stream} onOpenPage={onOpenPage} />;
}
