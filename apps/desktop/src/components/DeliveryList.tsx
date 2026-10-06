/// The events oxplow couldn't deliver and the effect reactions that
/// failed, each with what to do about it: Retry, or Discard (confirmed
/// inline). Shown in Settings → Data → Delivery and on the Alerts page
/// (tsk1097). Failures land in opErrorsStore.

import type { CSSProperties } from "react";
import { useState } from "react";

import { discardDeadLetter, retryDeadLetter, runCommand } from "../api.js";
import {
  letterLine,
  reactionLine,
  useFailedReactions,
  useUndelivered,
  type FailedReaction,
  type UndeliveredEvent,
} from "../delivery.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function DeliveryList({ emptyLabel }: { emptyLabel?: string }) {
  const undelivered = useUndelivered();
  const failedReactions = useFailedReactions();
  const [busy, setBusy] = useState<string | null>(null);

  /** Retry or discard a dead letter; the list re-reads itself. */
  async function decide(l: UndeliveredEvent, retry: boolean) {
    setBusy(`letter-${l.id}`);
    try {
      const after = retry ? await retryDeadLetter(l.id) : await discardDeadLetter(l.id);
      if (retry) {
        showToast({ message: after.state === "retried" ? `Delivered event ${l.eventSeq} to ${l.consumer}.` : `${l.consumer} failed on it again.` });
      }
    } catch (e) {
      recordOpError({ label: `${retry ? "Retry" : "Discard"} event ${l.eventSeq} for ${l.consumer}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  /** Have an effect react again to an event it failed on: the person's
   *  second click was the confirmation `effect.retry` asks for. */
  async function retryReaction(r: FailedReaction) {
    setBusy(`reaction-${r.effect}-${r.eventId}`);
    try {
      const out = await runCommand("effect.retry", { effect: r.effect, event: `event:${r.eventId}` }, true);
      const result = out.result as { outcome?: string; reason?: string } | null;
      showToast({
        message:
          result?.outcome === "ok"
            ? `${r.effect} reacted to event ${r.eventSeq}.`
            : `${r.effect}: ${result?.outcome ?? "no outcome"}${result?.reason ? ` (${result.reason})` : ""}.`,
      });
    } catch (e) {
      recordOpError({ label: `Retry ${r.effect} on event ${r.eventSeq}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <>
      {failedReactions.map((r) => {
        const key = `${r.effect}-${r.eventId}`;
        return (
          <div key={key} data-testid={`reaction-row-${key}`} style={rowStyle}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span>{reactionLine(r)}</span>
              <span style={{ flex: 1 }} />
              <InlineConfirm
                triggerLabel="Retry"
                confirmLabel="Retry"
                testIdPrefix={`reaction-retry-${key}`}
                title="Run the effect on this event again, as it is now. If the failed attempt was interrupted, a step outside oxplow may already have run — retrying sends it again."
                disabled={busy !== null}
                onConfirm={() => void retryReaction(r)}
              />
            </div>
            <div style={errorStyle}>{r.reason}</div>
          </div>
        );
      })}
      {undelivered.length === 0 && failedReactions.length === 0 && emptyLabel ? (
        <div style={mutedStyle} data-testid="data-delivery-empty">
          {emptyLabel}
        </div>
      ) : null}
      {undelivered.map((l) => (
        <div key={l.id} data-testid={`delivery-row-${l.id}`} style={rowStyle}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span>{letterLine(l)}</span>
            <span style={{ flex: 1 }} />
            <button
              type="button"
              data-testid={`delivery-retry-${l.id}`}
              title="Run the event through its consumer again"
              disabled={busy !== null}
              onClick={() => void decide(l, true)}
            >
              {busy === `letter-${l.id}` ? "Retrying…" : "Retry"}
            </button>
            <InlineConfirm
              triggerLabel="Discard"
              confirmLabel="Discard"
              testIdPrefix={`delivery-discard-${l.id}`}
              title="Give up on this event for this consumer"
              disabled={busy !== null}
              onConfirm={() => void decide(l, false)}
            />
          </div>
          <div style={errorStyle}>{l.error}</div>
        </div>
      ))}
    </>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const rowStyle: CSSProperties = { padding: "6px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)", marginTop: 4 };
