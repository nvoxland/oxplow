import type { ComponentProps } from "react";
import { useEffect, useState } from "react";
import { Page } from "../tabs/Page.js";
import { EditorPane } from "../components/EditorPane.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import { usePageTitle, useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { fileRef, snapshotRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import type { SnapshotBacklinkEntry } from "../tabs/BacklinksList.js";
import { listFileSnapshotsForPath } from "../api.js";
import { logUi } from "../logger.js";
import { formatShortDateTime } from "../components/format.js";

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
 * optional nav bar above it. Backlinks come from the unified
 * `page_ref` graph: every wiki page, tasks, commit, or finding
 * that references this file's path appears in the dropdown.
 * Outbound is generally empty for files (the file itself doesn't
 * point at other pages today), but the slot is wired so a future
 * import-graph extractor would surface immediately.
 */
export function FilePage({ dirty, ...editorProps }: FilePageProps) {
  const path = editorProps.filePath ?? "";
  const basename = path.split("/").pop() ?? path;
  usePageTitle(basename ? `${dirty ? "● " : ""}${basename}` : "");
  const ctxNav = useOptionalPageNavigation();
  const ref = fileRef(path);
  const backlinkEntries = useBacklinks(ref);
  const outboundEntries = usePageOutbound(ref);
  const [snapshotEntries, setSnapshotEntries] = useState<SnapshotBacklinkEntry[]>([]);

  // Per-file snapshot history. Driven off `file_snapshot` rows for
  // this path — every prior capture of the file's content. The
  // dropdown lets the user open SnapshotDetailSlideover for any
  // historical version. Refetches on path change only; the
  // snapshot service emits events on every capture but FilePage
  // doesn't subscribe yet (small list, infrequent reads).
  useEffect(() => {
    if (!path) {
      setSnapshotEntries([]);
      return;
    }
    let cancelled = false;
    void listFileSnapshotsForPath(path)
      .then((rows) => {
        if (cancelled) return;
        setSnapshotEntries(
          rows.map((row) => ({
            kind: "snapshot" as const,
            snapshotId: String(row.snapshotId ?? row.id),
            label: row.snapshotId
              ? `Snapshot ${row.snapshotId}`
              : `file_snapshot ${row.id}`,
            subtitle: formatShortDateTime(row.capturedAt),
          })),
        );
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "file snapshot history fetch failed", {
          error: String(err),
          path,
        });
      });
    return () => {
      cancelled = true;
    };
  }, [path]);

  const onOpen = (r: Parameters<NonNullable<typeof ctxNav>["navigate"]>[0]) =>
    ctxNav?.navigate(r);
  const backlinks =
    backlinkEntries.length > 0
      ? {
          count: backlinkEntries.length,
          body: <BacklinksList entries={backlinkEntries} />,
        }
      : undefined;
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} />,
        }
      : undefined;
  const snapshots =
    snapshotEntries.length > 0
      ? {
          count: snapshotEntries.length,
          body: (
            <BacklinksList
              entries={[]}
              snapshotEntries={snapshotEntries}
              onOpenSnapshot={({ snapshotId }) => {
                const numeric = Number(snapshotId);
                if (Number.isFinite(numeric)) onOpen(snapshotRef(numeric));
              }}
            />
          ),
        }
      : undefined;
  return (
    <Page
      testId="page-file"
      kind="file"
      backlinks={backlinks}
      outbound={outbound}
      snapshots={snapshots}
      commentsNav={path ? <CommentNavigator targetKind="file" targetId={path} /> : undefined}
    >
      <EditorPane {...editorProps} />
    </Page>
  );
}
