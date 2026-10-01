import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { InlinePromptStrip } from "../components/InlinePromptStrip.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import { LensSlots } from "../lens/LensSlots.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { personCommands } from "../personCommands.js";
import { useRequestGuard } from "../request-guard.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { workItemTabRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import {
  CANONICAL_STATES,
  featuresFor,
  readCapabilityProviders,
  readWorkItem,
  transitionWorkItem,
  workItemCommand,
  type CanonicalState,
  type WorkItem,
  type WorkItemsFeatures,
} from "../workItems.js";
import { recordOpError } from "../components/opErrorsStore.js";

const LABEL: Record<CanonicalState, string> = {
  todo: "To Do",
  in_progress: "In Progress",
  blocked: "Blocked",
  done: "Done",
  canceled: "Canceled",
};

/** What Link… proposes; the provider names its own link types. */
const DEFAULT_LINK_TYPE = "relates_to";

const NO_FEATURES: WorkItemsFeatures = { hierarchy: false, comments: false, links: false, inProgressOpensEffort: false };

/**
 * Another provider's work item (P6b.C3; oxplow's own open as `TaskPage`):
 * its title, state and body from `v_work_item`, Move To through the
 * provider, and — only where the provider declares the feature
 * (`v_capability_provider`) — its parent, Comment… and Link…. Extensions
 * mount lenses in the `work_item.detail.body` and `.sidebar` slots with
 * `{ ref, task_id: null }`.
 */
export function WorkItemPage({
  workItemRef,
  streamId,
  onOpenPage,
}: {
  workItemRef: string;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [item, setItem] = useState<WorkItem | null>(null);
  const [features, setFeatures] = useState<WorkItemsFeatures>(NO_FEATURES);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [loaded, setLoaded] = useState(false);
  const guard = useRequestGuard();
  const refresh = useCallback(async () => {
    // A newer read (another ref, or a re-run) wins over an older answer.
    const current = guard.begin();
    try {
      const [one, providers] = await Promise.all([readWorkItem(workItemRef), readCapabilityProviders("work_items")]);
      if (!current()) return;
      setItem(one.item);
      setFeatures(one.item ? featuresFor(providers.providers, one.item.provider) : NO_FEATURES);
      setReads(unionReads([one.reads, providers.reads]));
    } catch (e) {
      if (!current()) return;
      recordOpError({ label: "Load the work item", message: e instanceof Error ? e.message : String(e) });
    }
    setLoaded(true);
  }, [workItemRef, guard]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  const slotParams = useMemo(() => ({ ref: workItemRef, task_id: null }), [workItemRef]);
  usePageTitle(item?.title ?? null);
  const graphRef = useMemo(() => workItemTabRef(workItemRef), [workItemRef]);
  const backlinkEntries = useBacklinks(graphRef);
  const outboundEntries = usePageOutbound(graphRef);
  const [prompt, setPrompt] = useState<"comment" | "link" | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async (label: string, verb: "comment" | "link", input: Record<string, unknown>) => {
    if (!item) return;
    setBusy(true);
    const ran = await personCommands.run(label, workItemCommand(item.ref, verb), { ref: item.ref, ...input });
    setBusy(false);
    if (ran) setPrompt(null);
  };

  if (!item) {
    return (
      <Page testId="work-item-page" title={workItemRef} kind="work_item">
        <div style={mutedStyle}>{loaded ? "This work item isn't in any provider's list." : "Loading…"}</div>
      </Page>
    );
  }

  const rail = (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--text-sm)" }}>
      <div>
        <div style={labelStyle}>Provider</div>
        <div>{item.provider}</div>
      </div>
      <div>
        <div style={labelStyle}>State</div>
        <div>
          {LABEL[item.state]} <span style={mutedInline}>({item.nativeState})</span>
        </div>
      </div>
      {features.hierarchy && item.parentRef ? (
        <div data-testid="work-item-parent">
          <div style={labelStyle}>Parent</div>
          <RouteLink to={workItemTabRef(item.parentRef)} onNavigate={() => onOpenPage(workItemTabRef(item.parentRef!))} style={linkStyle}>
            {item.parentRef}
          </RouteLink>
        </div>
      ) : null}
      <div>
        <div style={labelStyle}>Move To</div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
          {CANONICAL_STATES.filter((s) => s !== item.state).map((s) => (
            <button
              key={s}
              type="button"
              data-testid={`work-item-move-${s}`}
              style={buttonStyle}
              onClick={() =>
                void transitionWorkItem(item.ref, s).catch((e: unknown) =>
                  recordOpError({ label: `Move to ${LABEL[s]}`, message: e instanceof Error ? e.message : String(e) }),
                )
              }
            >
              {LABEL[s]}
            </button>
          ))}
        </div>
      </div>
      <LensSlots slot="work_item.detail.sidebar" params={slotParams} streamId={streamId} onOpenPage={onOpenPage} />
    </div>
  );

  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };
  const outbound =
    outboundEntries.length > 0
      ? { count: outboundEntries.length, body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} /> }
      : undefined;

  return (
    <Page
      testId="work-item-page"
      title={item.title}
      kind="work_item"
      layout="details"
      rightRail={rail}
      backlinks={backlinks}
      outbound={outbound}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
        {item.body ? <MarkdownView body={item.body} /> : <div style={mutedStyle}>No description.</div>}
        {prompt === "comment" ? (
          <InlinePromptStrip
            testId="work-item-comment"
            message={`A comment on this item, sent to ${item.provider}. Cmd/Ctrl+Enter sends it.`}
            fields={[{ key: "body", placeholder: "A comment for the provider", multiline: true }]}
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
        ) : (
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
        )}
        <LensSlots slot="work_item.detail.body" params={slotParams} streamId={streamId} onOpenPage={onOpenPage} />
      </div>
    </Page>
  );
}

const mutedStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-sm)" };
const mutedInline: CSSProperties = { color: "var(--text-secondary)" };
const labelStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginBottom: 2 };
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  font: "inherit",
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
