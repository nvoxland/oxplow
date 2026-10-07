import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";

import type { EffortDetail, Stream, Thread } from "../api.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { InlinePromptStrip } from "../components/InlinePromptStrip.js";
import { ActivityTimeline, TaskDetail, TaskDetailRail } from "../components/Plan/TaskDetail.js";
import { Replaceable } from "../components/Replaceable.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { LensSlots } from "../lens/LensSlots.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { personCommands } from "../personCommands.js";
import { useRequestGuard } from "../request-guard.js";
import { BacklinksList, type SnapshotBacklinkEntry } from "../tabs/BacklinksList.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { gitCommitRef, snapshotRef, workItemTabRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { workItemId, workItemLabel } from "../workItemRef.js";
import {
  applyItemChange,
  deleteWorkItem,
  featuresFor,
  moveWorkItem,
  NO_FEATURES,
  readCapabilityProviders,
  readItemEfforts,
  readWorkItem,
  type FieldDecl,
  type ItemChange,
  type WorkItem,
  type WorkItemsFeatures,
} from "../workItems.js";

/** What Link… proposes; the provider names its own link types. */
const DEFAULT_LINK_TYPE = "relates_to";

/**
 * A work item's page, whichever list it's on: its title and body (edited
 * in place), its state and the list's own fields (as it declares them) in
 * the rail, its activity (the efforts on it), and — only where its list
 * declares the feature (`v_capability_provider`) — its parent, the move
 * between a thread and the backlog, Comment…, Link… and Delete. Every
 * write is a `work_item.*` command, which the bus dispatches to the item's
 * list. Extensions mount lenses in the `work_item.detail.body` and
 * `.sidebar` slots with `{ ref }`; the item's own list may replace how its
 * state shows (`work_item.detail.state`).
 */
export function WorkItemPage({
  workItemRef,
  stream,
  thread,
  onOpenPage,
  onOpenFile,
  onShowEffortDiff,
  onOpenDiff,
  onDeleted,
}: {
  workItemRef: string;
  stream: Stream | null;
  /** The thread the person is on: "Bring to this thread" moves it here. */
  thread: Thread | null;
  onOpenPage(ref: TabRef): void;
  onOpenFile?(path: string): void;
  /** Open the diff view for an effort (start→end snapshots). */
  onShowEffortDiff?(effortId: string): void;
  onOpenDiff?(spec: DiffSpec): void;
  /** The item was deleted (the host closes the tab). */
  onDeleted?(ref: string): void;
}) {
  const streamId = stream?.id ?? null;
  const [item, setItem] = useState<WorkItem | null>(null);
  const [features, setFeatures] = useState<Required<WorkItemsFeatures>>(NO_FEATURES);
  const [fields, setFields] = useState<FieldDecl[]>([]);
  const [efforts, setEfforts] = useState<EffortDetail[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [loaded, setLoaded] = useState(false);
  const guard = useRequestGuard();
  const refresh = useCallback(async () => {
    // A newer read (another ref, or a re-run) wins over an older answer.
    const current = guard.begin();
    try {
      const [one, providers, activity] = await Promise.all([
        readWorkItem(workItemRef),
        readCapabilityProviders("work_items"),
        readItemEfforts(workItemRef),
      ]);
      if (!current()) return;
      setItem(one.item);
      const provider = one.item?.provider;
      setFeatures(provider ? featuresFor(providers.providers, provider) : NO_FEATURES);
      setFields(providers.providers.find((p) => p.provider === provider)?.fields ?? []);
      setEfforts(activity.efforts);
      // An effort opens, closes or links as the effort policy reacts to a
      // state move: the Activity timeline re-reads without a remount.
      setReads(unionReads([one.reads, providers.reads, activity.reads]));
    } catch (e) {
      if (!current()) return;
      recordOpError({ label: "Load the work item", message: e instanceof Error ? e.message : String(e) });
    }
    setLoaded(true);
  }, [workItemRef, guard]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  const slotParams = useMemo(() => ({ ref: workItemRef }), [workItemRef]);
  usePageTitle(item?.title ?? null);
  const graphRef = useMemo(() => workItemTabRef(workItemRef), [workItemRef]);
  const backlinkEntries = useBacklinks(graphRef);
  const outboundEntries = usePageOutbound(graphRef);
  const [prompt, setPrompt] = useState<"comment" | "link" | null>(null);
  const [busy, setBusy] = useState(false);

  const update = async (ref: string, change: ItemChange) => {
    try {
      await applyItemChange(ref, change);
    } catch (e) {
      recordOpError({ label: "Edit the work item", message: e instanceof Error ? e.message : String(e) });
    }
  };
  // The inline confirm is the person's confirmation of the destructive run.
  const remove = async () => {
    if (!item) return;
    try {
      await deleteWorkItem(item.ref, true);
      onDeleted?.(item.ref);
    } catch (e) {
      recordOpError({ label: "Delete", message: e instanceof Error ? e.message : String(e) });
    }
  };
  const run = async (label: string, verb: "comment" | "link", input: Record<string, unknown>) => {
    if (!item) return;
    setBusy(true);
    const ran = await personCommands.run(label, `oxplow.work_item.${verb}`, { ref: item.ref, ...input });
    setBusy(false);
    if (ran) setPrompt(null);
  };

  const snapshotBacklinks = useMemo<SnapshotBacklinkEntry[]>(
    () =>
      efforts
        .filter((d) => !!d.effort.end_snapshot_id)
        .map((d, i) => ({
          kind: "snapshot" as const,
          snapshotId: d.effort.end_snapshot_id!,
          label: `Effort ${i + 1} end snapshot`,
          source: "effort-end",
          snapshotLabel: null,
          subtitle: `${d.changed_paths.length} file${d.changed_paths.length === 1 ? "" : "s"}`,
        })),
    [efforts],
  );
  const backlinks = {
    count: backlinkEntries.length + snapshotBacklinks.length,
    body: (
      <BacklinksList
        entries={backlinkEntries}
        snapshotEntries={snapshotBacklinks}
        onOpenPage={onOpenPage}
        onOpenSnapshot={(payload) => {
          const id = Number(payload.snapshotId);
          if (Number.isFinite(id)) onOpenPage(snapshotRef(id));
        }}
        onOpenCommit={(payload) => onOpenPage(gitCommitRef(payload.sha))}
      />
    ),
  };
  const outbound =
    outboundEntries.length > 0
      ? { count: outboundEntries.length, body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} /> }
      : undefined;

  if (!item) {
    return (
      <Page testId="work-item-page" title={workItemLabel(workItemRef)} kind="work_item" backlinks={backlinks} outbound={outbound}>
        <div style={mutedStyle}>{loaded ? "This work item isn't on the active work list." : "Loading…"}</div>
      </Page>
    );
  }

  // A list that keeps items on threads moves one between a thread and the
  // backlog.
  const scopeAction = (() => {
    if (!features.lists || !thread) return undefined;
    if (item.threadId === null) {
      return { label: "Bring to this thread", run: () => void moveWorkItem(item.ref, thread.id) };
    }
    if (item.threadId === thread.id) return { label: "Send to backlog", run: () => void moveWorkItem(item.ref, null) };
    return undefined;
  })();
  // The newest effort (they come newest first) is the one to review.
  const latestEffort = efforts[0]?.effort.id;
  const review = onShowEffortDiff && latestEffort ? () => onShowEffortDiff(latestEffort) : undefined;
  const commentTarget = workItemId(item.ref) ?? item.ref;

  const rail = (
    <>
      <TaskDetailRail
        item={item}
        fields={fields}
        onUpdateTask={update}
        onDelete={features.delete ? () => void remove() : undefined}
        onReview={review}
        scopeAction={scopeAction}
        extra={
          <>
            {/* The item's own list's extension may show its state its own
                way, given the ref; the state pill above stays — whatever a
                replacement offers, the item can always move. */}
            <Replaceable
              target="work_item.detail.state"
              props={{ ref: item.ref }}
              streamId={streamId}
              provider={item.provider}
              onOpenPage={onOpenPage}
              fallback={null}
            />
            {features.hierarchy && item.parentRef ? (
            <div data-testid="work-item-parent">
              <div style={labelStyle}>Parent</div>
              <RouteLink
                to={workItemTabRef(item.parentRef)}
                onNavigate={() => onOpenPage(workItemTabRef(item.parentRef!))}
                style={linkStyle}
              >
                {workItemLabel(item.parentRef)}
              </RouteLink>
            </div>
            ) : null}
          </>
        }
      />
      <LensSlots slot="work_item.detail.sidebar" params={slotParams} streamId={streamId} onOpenPage={onOpenPage} />
    </>
  );

  return (
    <Page
      testId="work-item-page"
      title={item.title}
      kind="work_item"
      layout="details"
      rightRail={rail}
      backlinks={backlinks}
      outbound={outbound}
      commentsNav={stream ? <CommentNavigator targetKind="work_item" targetId={commentTarget} /> : undefined}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
        <TaskDetail
          item={item}
          onUpdateTask={update}
          comments={
            stream
              ? { streamId: stream.id, threadId: item.threadId, targetKind: "work_item", targetId: commentTarget }
              : undefined
          }
        />
        {prompt === "comment" ? (
          <InlinePromptStrip
            testId="work-item-comment"
            message={`A comment on this item, sent to ${item.provider}. Cmd/Ctrl+Enter sends it.`}
            fields={[{ key: "body", placeholder: "A comment for the list", multiline: true }]}
            confirmLabel="Comment"
            busy={busy}
            onSubmit={({ body }) => void run("Comment", "comment", { body })}
            onCancel={() => setPrompt(null)}
          />
        ) : prompt === "link" ? (
          <InlinePromptStrip
            testId="work-item-link"
            message={`Link this item to another, as ${item.provider} names the link.`}
            fields={[
              { key: "link_type", initialValue: DEFAULT_LINK_TYPE, placeholder: "link type" },
              { key: "target", placeholder: "work_item:<provider>:<id>" },
            ]}
            confirmLabel="Link"
            busy={busy}
            onSubmit={({ link_type, target }) => void run("Link", "link", { target, link_type })}
            onCancel={() => setPrompt(null)}
          />
        ) : features.comments || features.links ? (
          <div style={{ display: "flex", gap: 12 }}>
            {features.comments ? (
              <button type="button" data-testid="work-item-comment-open" style={buttonStyle} onClick={() => setPrompt("comment")}>
                Comment…
              </button>
            ) : null}
            {features.links ? (
              <button type="button" data-testid="work-item-link-open" style={buttonStyle} onClick={() => setPrompt("link")}>
                Link…
              </button>
            ) : null}
          </div>
        ) : null}
        <LensSlots
          slot="work_item.detail.body"
          params={slotParams}
          streamId={streamId}
          onOpenPage={onOpenPage}
          h2ClassName="task-activity-heading"
        />
        <section>
          <h2 className="task-activity-heading">Activity</h2>
          <ActivityTimeline
            efforts={efforts}
            formatTimestamp={(iso) => new Date(iso).toLocaleString()}
            onOpenFile={onOpenFile}
            onShowEffortDiff={onShowEffortDiff}
            onOpenDiff={onOpenDiff}
          />
        </section>
      </div>
    </Page>
  );
}

const mutedStyle: CSSProperties = { padding: "16px 20px", color: "var(--text-secondary)", fontSize: "var(--text-sm)" };
const labelStyle: CSSProperties = {
  textTransform: "uppercase",
  letterSpacing: 0.5,
  fontSize: 10,
  color: "var(--text-secondary)",
  fontWeight: 500,
  marginBottom: 4,
};
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  font: "inherit",
  fontSize: "var(--text-xs)",
};
const buttonStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  padding: "2px 8px",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  background: "transparent",
  color: "var(--text-primary)",
  cursor: "pointer",
};
