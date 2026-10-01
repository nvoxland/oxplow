import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import { LensSlots } from "../lens/LensSlots.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { personCommands } from "../personCommands.js";
import { Page } from "../tabs/Page.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { workItemTabRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
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

/** Link types a work item link may name. */
const LINK_TYPES = ["relates_to", "blocks", "duplicates", "supersedes", "discovered_from", "replies_to"];

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
  const refresh = useCallback(async () => {
    try {
      const [one, providers] = await Promise.all([readWorkItem(workItemRef), readCapabilityProviders("work_items")]);
      setItem(one.item);
      setFeatures(one.item ? featuresFor(providers.providers, one.item.provider) : NO_FEATURES);
      setReads(unionReads([one.reads, providers.reads]));
    } catch (e) {
      recordOpError({ label: "Load the work item", message: e instanceof Error ? e.message : String(e) });
    } finally {
      setLoaded(true);
    }
  }, [workItemRef]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  const slotParams = useMemo(() => ({ ref: workItemRef, task_id: null }), [workItemRef]);

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

  return (
    <Page testId="work-item-page" title={item.title} kind="work_item" layout="details" rightRail={rail}>
      <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
        {item.body ? <MarkdownView body={item.body} /> : <div style={mutedStyle}>No description.</div>}
        <div style={{ display: "flex", gap: 12 }}>
          {features.comments ? (
            <InlineCommand
              testId="work-item-comment"
              label="Comment…"
              placeholder="A comment for the provider"
              multiline
              run={(text) =>
                personCommands.run("Comment", workItemCommand(item.ref, "comment"), { ref: item.ref, body: text })
              }
            />
          ) : null}
          {features.links ? (
            <LinkForm
              run={(target, linkType) =>
                personCommands.run("Link", workItemCommand(item.ref, "link"), {
                  ref: item.ref,
                  target,
                  link_type: linkType,
                })
              }
            />
          ) : null}
        </div>
        <LensSlots slot="work_item.detail.body" params={slotParams} streamId={streamId} onOpenPage={onOpenPage} />
      </div>
    </Page>
  );
}

/** A button that opens an inline field; Cmd/Ctrl+Enter (or the submit
 *  button) runs it, Escape closes it. The text stays until it ran. */
function InlineCommand({
  testId,
  label,
  placeholder,
  multiline,
  run,
}: {
  testId: string;
  label: string;
  placeholder: string;
  multiline?: boolean;
  run(text: string): Promise<boolean>;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!open) {
    return (
      <button type="button" data-testid={`${testId}-open`} style={buttonStyle} onClick={() => setOpen(true)}>
        {label}
      </button>
    );
  }
  const submit = async () => {
    if (!text.trim()) return;
    setBusy(true);
    const ran = await run(text.trim());
    setBusy(false);
    if (ran) {
      setText("");
      setOpen(false);
    }
  };
  return (
    <form
      style={{ display: "flex", flexDirection: "column", gap: 4, flex: 1 }}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      {multiline ? (
        <textarea
          data-testid={`${testId}-input`}
          autoFocus
          value={text}
          placeholder={placeholder}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setOpen(false);
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
          }}
          style={inputStyle}
          rows={3}
        />
      ) : null}
      <div style={{ display: "flex", gap: 6 }}>
        <button type="submit" data-testid={`${testId}-submit`} disabled={busy || !text.trim()} style={buttonStyle}>
          {label.replace("…", "")}
        </button>
        <button type="button" style={buttonStyle} onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </form>
  );
}

/** Link… : the target ref and the link type; Enter submits. */
function LinkForm({ run }: { run(target: string, linkType: string): Promise<boolean> }) {
  const [open, setOpen] = useState(false);
  const [target, setTarget] = useState("");
  const [linkType, setLinkType] = useState(LINK_TYPES[0]!);
  if (!open) {
    return (
      <button type="button" data-testid="work-item-link-open" style={buttonStyle} onClick={() => setOpen(true)}>
        Link…
      </button>
    );
  }
  return (
    <form
      style={{ display: "flex", gap: 6, alignItems: "center" }}
      onSubmit={(e) => {
        e.preventDefault();
        if (!target.trim()) return;
        void run(target.trim(), linkType).then((ran) => {
          if (ran) setOpen(false);
        });
      }}
    >
      <select data-testid="work-item-link-type" value={linkType} onChange={(e) => setLinkType(e.target.value)} style={inputStyle}>
        {LINK_TYPES.map((t) => (
          <option key={t} value={t}>
            {t.replace(/_/g, " ")}
          </option>
        ))}
      </select>
      <input
        data-testid="work-item-link-target"
        autoFocus
        value={target}
        placeholder="work_item:<provider>:<id>"
        onChange={(e) => setTarget(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setOpen(false);
        }}
        style={inputStyle}
      />
      <button type="submit" disabled={!target.trim()} style={buttonStyle}>
        Link
      </button>
    </form>
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
const inputStyle: CSSProperties = {
  fontSize: "var(--text-sm)",
  padding: "4px 6px",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  background: "var(--surface-input, var(--surface-card))",
  color: "var(--text-primary)",
  fontFamily: "inherit",
};
