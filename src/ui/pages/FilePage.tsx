import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { EditorPane } from "../components/EditorPane.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";

export interface FilePageProps extends ComponentProps<typeof EditorPane> {
  /** True when the file's draft differs from saved content. Drives the
   *  ● dirty marker on the page title. */
  dirty: boolean;
}

/**
 * Thin Page wrapper around `EditorPane` so file tabs share the same
 * browser-style chrome as every other non-agent tab. The title comes
 * from the file's basename + dirty marker via `usePageTitle`.
 *
 * EditorPane keeps owning all of its internal toolbar / Monaco
 * decorations / blame overlay — the chrome only adds the title row +
 * optional nav bar above it.
 */
export function FilePage({ dirty, ...editorProps }: FilePageProps) {
  const path = editorProps.filePath ?? "";
  const basename = path.split("/").pop() ?? path;
  usePageTitle(basename ? `${dirty ? "● " : ""}${basename}` : "");
  return (
    <Page testId="page-file" kind="file">
      <EditorPane {...editorProps} />
    </Page>
  );
}
