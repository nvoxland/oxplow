/// The Alerts dock panel: what needs the person. A toolbar in the nav bar's
/// colors (`DockToolbar`) holds **Clear All** on the left (the notices and
/// failed operations: what a person can put away, `clearable`) and how many
/// of each kind wait on the right. Below, one row per alert — decisions,
/// then failures, then notices, newest first, a failure that repeated merged
/// (`alertRows`): a chevron, the kind's icon, the title, what it says on one
/// line (cut to fit), ×N, its age, and a × that dismisses it when it can be.
/// A row's click opens it in place (as the Context panel's): the whole
/// message, the command and the time, a failed operation's captured output,
/// and what can be done about it: a decision's Approve / Decline, undelivered events' Retry All / Discard
/// All, failed reactions' Retry All, an extension problem's Repair with the
/// Agent; Open (only when it has a page of its own) and Copy. The icon's badge is the same count the bell showed
/// (`alertsSummary`). The Alerts page is the panel header's arrow.

import { ChevronRight, CircleAlert, CircleHelp, Info, X } from "lucide-react";
import { useEffect, useState, type CSSProperties, type KeyboardEvent } from "react";

import { dismissAllHints, dismissHint } from "../../hints.js";
import { dismissAllOpErrors, dismissOpError, readOpErrorOutput, type OpError } from "../../opErrors.js";
import { decide, type Proposal } from "../../proposals.js";
import { insertIntoAgent } from "../../agent-input-bus.js";
import { personCommands } from "../../personCommands.js";
import { dockStore } from "../Dock/dockStore.js";
import { refFromTabId } from "../../tabs/pageRefs.js";
import { InlineConfirm } from "../InlineConfirm.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { BarButton, DockToolbar } from "../Dock/DockToolbar.js";
import { formatFullDateTime } from "../format.js";
import { alertCounts, alertRows, alertsSummary, clearable, shortAge, type AlertItems, type AlertKind, type AlertRow } from "./alertsModel.js";
import { useAlerts } from "./useAlerts.js";
import { copyText } from "../../clipboard.js";

export function useAlertsBadge(): DockBadge | null {
  const { items } = useAlerts();
  const { count, tone } = alertsSummary(items);
  return count > 0 ? { count, ...(tone === "danger" ? { tone } : {}) } : null;
}

/** Put away every notice and every failed operation at once. */
async function clearAlerts(items: AlertItems): Promise<void> {
  await Promise.all([...(items.opErrors.length > 0 ? [dismissAllOpErrors()] : []), ...(items.hints.length > 0 ? [dismissAllHints()] : [])]);
}

/** What the toolbar says waits: "1 decision · 3 failed · 2 notices". */
export function countsLine(counts: Record<AlertKind, number>): string {
  const parts: string[] = [];
  if (counts.decision) parts.push(`${counts.decision} ${counts.decision === 1 ? "decision" : "decisions"}`);
  if (counts.failure) parts.push(`${counts.failure} failed`);
  if (counts.notice) parts.push(`${counts.notice} ${counts.notice === 1 ? "notice" : "notices"}`);
  return parts.join(" · ");
}

export function AlertsDockBody({ onOpenPage }: DockBodyProps) {
  const alerts = useAlerts();
  const { items } = alerts;
  const rows = alertRows({
    proposals: alerts.proposals,
    opErrors: alerts.opErrors,
    undelivered: items.undelivered,
    failedReactions: items.failedReactions,
    extensionProblems: items.extensionProblems,
    badges: alerts.badges,
    hints: alerts.hints,
  });
  const [expanded, setExpanded] = useState<string | null>(null);
  return (
    <div data-testid="alerts-panel" style={{ display: "flex", flexDirection: "column", fontSize: "var(--text-xs)", minHeight: "100%" }}>
      <DockToolbar testId="alerts-toolbar">
        <BarButton data-testid="alerts-panel-clear" disabled={!clearable(items)} title="Dismiss the notices and the failed operations" onClick={() => void clearAlerts(items)}>
          Clear All
        </BarButton>
        <span data-testid="alerts-counts" style={{ marginLeft: "auto", color: "var(--text-muted)" }}>
          {countsLine(alertCounts(rows))}
        </span>
      </DockToolbar>
      <div style={{ padding: "0 12px 8px" }}>
        {rows.length === 0 ? (
          <div data-testid="alerts-panel-empty" style={{ color: "var(--text-muted)", padding: "8px 0" }}>
            Nothing needs you.
          </div>
        ) : (
          <div role="list">
            {rows.map((row) => (
              <AlertRowView
                key={row.key}
                row={row}
                expanded={expanded === row.key}
                onToggle={() => setExpanded((now) => (now === row.key ? null : row.key))}
                proposal={alerts.proposals.find((p) => p.uri === row.proposal) ?? null}
                opError={newestOf(alerts.opErrors, row.opErrorIds)}
                onOpenPage={onOpenPage}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** The newest of the failed operations a row stands for. */
function newestOf(errors: readonly OpError[], ids: readonly string[] | undefined): OpError | null {
  if (!ids) return null;
  return errors.filter((e) => ids.includes(e.id)).reduce<OpError | null>((best, e) => (!best || e.at > best.at ? e : best), null);
}

const KIND_ICON: Record<AlertKind, { Icon: typeof CircleAlert; color: string; label: string }> = {
  decision: { Icon: CircleHelp, color: "var(--accent)", label: "Waiting on your decision" },
  failure: { Icon: CircleAlert, color: "var(--severity-critical)", label: "Failed" },
  notice: { Icon: Info, color: "var(--text-muted)", label: "Notice" },
};

function AlertRowView({
  row,
  expanded,
  onToggle,
  proposal,
  opError,
  onOpenPage,
}: {
  row: AlertRow;
  expanded: boolean;
  onToggle(): void;
  proposal: Proposal | null;
  opError: OpError | null;
  onOpenPage: DockBodyProps["onOpenPage"];
}) {
  const [hover, setHover] = useState(false);
  // The captured output of the newest failure the row stands for, by its id:
  // a newer failure of the same kind reads its own, not the first's.
  const [read, setRead] = useState<{ id: string; text: string } | null>(null);
  const { Icon, color, label } = KIND_ICON[row.kind];
  const dismiss = row.opErrorIds ? () => Promise.all(row.opErrorIds!.map(dismissOpError)) : row.hintId != null ? () => dismissHint(row.hintId!) : null;
  const openRef = row.open ? refFromTabId(row.open) : null;

  // A failed operation's captured output, read when its row first opens.
  const outputId = expanded && opError?.outputSize ? opError.id : null;
  useEffect(() => {
    if (!outputId) return;
    let live = true;
    void readOpErrorOutput(outputId).then((out) => {
      if (live) setRead({ id: outputId, text: [out?.stderr, out?.stdout].filter(Boolean).join("\n") || "" });
    });
    return () => {
      live = false;
    };
  }, [outputId]);
  const output = read && read.id === outputId ? read.text : null;

  const keys = (e: KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onToggle();
    }
  };
  const whole = [row.detail, output].filter(Boolean).join("\n\n");
  return (
    <div
      role="listitem"
      onKeyDown={(e) => {
        if (e.key === "Escape" && expanded) {
          e.preventDefault();
          e.stopPropagation();
          onToggle();
        }
      }}
      style={{ margin: "0 -6px", borderBottom: "1px solid var(--border-subtle)", background: expanded ? "var(--button-secondary-bg-hover)" : "transparent" }}
    >
      <div
        tabIndex={0}
        aria-expanded={expanded}
        data-testid="alerts-panel-item"
        data-kind={row.kind}
        onClick={onToggle}
        onKeyDown={keys}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{ display: "flex", alignItems: "center", gap: 8, padding: "5px 6px", background: hover ? "var(--button-secondary-bg-hover)" : "transparent", cursor: "pointer", minWidth: 0 }}
      >
        <ChevronRight size={12} aria-hidden style={{ flexShrink: 0, color: "var(--text-muted)", transform: expanded ? "rotate(90deg)" : "none", transition: "transform 150ms ease" }} />
        <Icon size={13} aria-label={label} style={{ flexShrink: 0, color }} />
        <span data-testid="alerts-panel-title" style={{ ...ellipsis, flex: "0 1 auto", maxWidth: "45%", fontWeight: 600 }}>
          {row.title}
        </span>
        <span data-testid="alerts-panel-detail" style={{ ...ellipsis, flex: "1 1 0", color: "var(--text-secondary)" }}>
          {row.detail ? row.detail.replace(/\s+/g, " ").trim() : ""}
        </span>
        {row.count > 1 ? (
          <span data-testid="alerts-panel-count" title={`${row.count} times`} style={{ flexShrink: 0, color: "var(--text-muted)" }}>
            ×{row.count}
          </span>
        ) : null}
        {row.at != null ? (
          <span data-testid="alerts-panel-age" title={formatFullDateTime(new Date(row.at).toISOString())} style={{ flexShrink: 0, color: "var(--text-muted)", minWidth: 24, textAlign: "right" }}>
            {shortAge(row.at)}
          </span>
        ) : null}
        {dismiss ? (
          <button
            type="button"
            data-testid="alerts-panel-dismiss"
            aria-label="Dismiss"
            title={row.count > 1 ? `Dismiss all ${row.count}` : "Dismiss"}
            onClick={(e) => {
              e.stopPropagation();
              void dismiss();
            }}
            style={iconButton}
          >
            <X size={14} aria-hidden />
          </button>
        ) : (
          <span style={{ width: 14, flexShrink: 0 }} />
        )}
      </div>
      <div style={{ display: "grid", gridTemplateRows: expanded ? "1fr" : "0fr", transition: "grid-template-rows 180ms ease" }}>
        <div data-testid="alerts-item-details" inert={!expanded} aria-hidden={!expanded} style={{ overflow: "hidden", minHeight: 0 }}>
          <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: "2px 6px 10px 47px" }}>
            {whole ? (
              <div data-testid="alerts-item-text" style={quoteStyle}>
                {whole}
              </div>
            ) : null}
            {row.command || row.at != null ? (
              <div style={{ color: "var(--text-muted)" }}>
                {row.command ? <code>{row.command}</code> : null}
                {row.command && row.at != null ? " · " : null}
                {row.at != null ? formatFullDateTime(new Date(row.at).toISOString()) : null}
              </div>
            ) : null}
            <div style={{ display: "flex", gap: 14, alignItems: "center" }}>
              {proposal ? (
                <>
                  {proposal.preview.destructive ? (
                    <InlineConfirm triggerLabel="Approve" confirmLabel="Approve" testIdPrefix="alerts-item-approve" onConfirm={() => void decide(proposal, true)} />
                  ) : (
                    <button type="button" data-testid="alerts-item-approve" onClick={() => void decide(proposal, true)} style={linkButton}>
                      Approve
                    </button>
                  )}
                  <button type="button" data-testid="alerts-item-decline" onClick={() => void decide(proposal, false)} style={linkButton}>
                    Decline
                  </button>
                </>
              ) : null}
              {row.source === "delivery" ? (
                <>
                  <button type="button" data-testid="alerts-item-retry" onClick={() => void personCommands.run("Retry Events", "oxplow.delivery.retry_all", {})} style={linkButton}>
                    Retry All
                  </button>
                  <button type="button" data-testid="alerts-item-discard" onClick={() => void personCommands.run("Discard Events", "oxplow.delivery.discard_all", {})} style={linkButton}>
                    Discard All…
                  </button>
                </>
              ) : null}
              {row.source === "reactions" ? (
                <button type="button" data-testid="alerts-item-retry" onClick={() => void personCommands.run("Retry Reactions", "oxplow.effect.retry_all", {})} style={linkButton}>
                  Retry All…
                </button>
              ) : null}
              {row.source === "extension" && row.extension ? (
                <button
                  type="button"
                  data-testid="alerts-item-repair"
                  title="Put a request to repair it in the agent's input (you send it)"
                  onClick={() => {
                    insertIntoAgent(`Repair the \`${row.extension}\` extension: ${row.detail ?? "it isn't running as it should"}`);
                    dockStore.close();
                  }}
                  style={linkButton}
                >
                  Repair with the Agent
                </button>
              ) : null}
              {openRef ? (
                <button type="button" data-testid="alerts-item-open" onClick={() => onOpenPage(openRef)} style={linkButton}>
                  Open
                </button>
              ) : null}
              {whole ? (
                <button type="button" data-testid="alerts-item-copy" onClick={() => void copyText([row.title, whole].join("\n\n"))} style={linkButton}>
                  Copy
                </button>
              ) : null}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

const ellipsis = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 } as const;

const quoteStyle: CSSProperties = {
  color: "var(--text-secondary)",
  borderLeft: "2px solid var(--border-subtle)",
  paddingLeft: 8,
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  fontFamily: "var(--font-mono, monospace)",
  maxHeight: 220,
  overflow: "auto",
};

const linkButton: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  font: "inherit",
  color: "var(--accent)",
  cursor: "pointer",
  textAlign: "left",
};

const iconButton = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-muted)",
  cursor: "pointer",
  display: "inline-flex",
  flexShrink: 0,
} as const;
