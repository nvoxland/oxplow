/// Everything that needs the person, in one place: opened from
/// the status bar's bell or a toast's Review. Three groups:
/// - Needs your decision — each waiting proposal as its card, with its
///   preview and Approve / Decline (the one place besides its thread
///   where a person consents);
/// - Problems — failed operations, each opening to its output, and the
///   events and reactions that couldn't be delivered, with Retry / Discard;
/// - From extensions — panel badges that fire, each opening its lens.

import type { CSSProperties, ReactNode } from "react";
import { useState } from "react";

import { useAlerts } from "../components/Alerts/useAlerts.js";
import { OpErrorDetail } from "../components/Alerts/OpErrorDetail.js";
import { alertsSummary } from "../components/Alerts/alertsModel.js";
import { DeliveryList } from "../components/DeliveryList.js";
import { getOpErrorsStore } from "../components/opErrorsStore.js";
import { ProposalCard } from "../components/Proposals/ProposalCard.js";
import { decide } from "../proposals.js";
import { Page } from "../tabs/Page.js";
import { lensRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

export function AlertsPage({ onOpenPage }: { onOpenPage(ref: TabRef): void }) {
  const { items, proposals, opErrors, badges } = useAlerts();
  const store = getOpErrorsStore();
  const [open, setOpen] = useState<string | null>(null);
  const problems = items.opErrors.length + items.undelivered + items.failedReactions;
  const { count } = alertsSummary(items);
  return (
    <Page testId="page-alerts" title="Alerts">
      <div style={{ padding: 16, display: "flex", flexDirection: "column", gap: 20, overflow: "auto" }}>
        {count === 0 ? (
          <div data-testid="alerts-empty" style={mutedStyle}>
            Nothing needs you.
          </div>
        ) : null}
        {proposals.length > 0 ? (
          <Section title="Needs your decision" testId="alerts-decisions">
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {proposals.map((p) => (
                <ProposalCard key={p.id} proposal={p} onDecide={decide} />
              ))}
            </div>
          </Section>
        ) : null}
        {problems > 0 ? (
          <Section
            title="Problems"
            testId="alerts-problems"
            action={
              opErrors.length > 0 ? (
                <button type="button" data-testid="alerts-clear-ops" onClick={() => store.clear()} title="Clear every failed operation">
                  Clear failed operations
                </button>
              ) : null
            }
          >
            {opErrors.map((e) => {
              const expanded = open === e.id;
              return (
                <div key={e.id} style={rowStyle}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <button
                      type="button"
                      data-testid={`alerts-op-${e.id}`}
                      aria-expanded={expanded}
                      onClick={() => {
                        store.markSeen(e.id);
                        setOpen(expanded ? null : e.id);
                      }}
                      title={expanded ? "Hide the output" : "Show the output"}
                      style={{ ...linkButtonStyle, color: e.seen ? "var(--text-secondary)" : "var(--severity-critical)" }}
                    >
                      {expanded ? "▾" : "▸"} {e.label}
                    </button>
                    <span style={{ flex: 1 }} />
                    <span style={mutedStyle}>{new Date(e.at).toLocaleTimeString()}</span>
                    <button
                      type="button"
                      data-testid={`alerts-op-dismiss-${e.id}`}
                      title="Dismiss"
                      onClick={() => store.dismiss(e.id)}
                      style={linkButtonStyle}
                    >
                      ×
                    </button>
                  </div>
                  {expanded ? <OpErrorDetail entry={e} /> : null}
                </div>
              );
            })}
            <DeliveryList />
          </Section>
        ) : null}
        {badges.length > 0 ? (
          <Section title="From extensions" testId="alerts-badges">
            {badges.map((b) => (
              <div key={b.id} style={rowStyle}>
                <button
                  type="button"
                  data-testid={`alerts-badge-${b.id}`}
                  onClick={() => onOpenPage(lensRef(b.id))}
                  title={`Open ${b.title}`}
                  style={linkButtonStyle}
                >
                  {b.title}
                </button>{" "}
                <span style={mutedStyle}>{b.message}</span>
              </div>
            ))}
          </Section>
        ) : null}
      </div>
    </Page>
  );
}

function Section({ title, testId, action, children }: { title: string; testId: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section data-testid={testId} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <h3 style={{ margin: 0, fontSize: "var(--text-sm)" }}>{title}</h3>
        <span style={{ flex: 1 }} />
        {action}
      </div>
      {children}
    </section>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const rowStyle: CSSProperties = { padding: "6px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const linkButtonStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  cursor: "pointer",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
  textAlign: "left",
};
