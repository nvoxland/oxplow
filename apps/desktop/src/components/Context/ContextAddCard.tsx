/// The card at the top of the Context dock panel while an Add to Context
/// waits (`addToContext` opens the panel): what is being added — by its
/// name, never a raw URI — an optional note of what the person wants the
/// agent to do about it, and **To**, the basket it goes in: This Thread (the
/// thread on screen's own), each named basket, or **+ New basket**, which
/// asks for a name. To starts at the basket last added to (an app
/// preference). Ctrl/Cmd+Enter or Add adds — with no note too; Enter is a
/// new line in the note, and adds from the new basket's name; Escape or
/// Cancel adds nothing (and is not passed on, or the dock would close). The
/// draft lives in the add's store, so the dock closing keeps it.

import { useEffect, useRef, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { FileText, Quote } from "lucide-react";

import { formatShortcut } from "../../keybindings.js";
import { useAppPreference } from "../../prefs/remembered.js";
import {
  LAST_BASKET_PREF,
  NEW_MARK,
  THREAD_BASKET,
  THREAD_MARK,
  cancelAddToContext,
  defaultDestination,
  rememberedAs,
  submitToContext,
  updatePendingAdd,
  type Baskets,
  type Destination,
  type PendingAdd,
} from "../../contextBasket.js";
import { useContextTitle, useResolvedTitle } from "./contextTitle.js";

/** The card's choice for `to`. */
function choiceOf(to: Destination | null): string {
  if (!to) return NEW_MARK;
  return "thread" in to ? THREAD_MARK : to.basket;
}

/** Where the card adds to now; null while nothing addable is picked. */
export function destinationOf(add: PendingAdd, choice: string): Destination | null {
  if (choice === THREAD_MARK) return add.thread ? { thread: add.thread } : null;
  if (choice === NEW_MARK) return add.newName.trim() ? { basket: add.newName.trim() } : null;
  return { basket: choice };
}

export function ContextAddCard({ add, baskets, onAdded }: { add: PendingAdd; baskets: Baskets; onAdded?(to: Destination): void }) {
  const [last, setLast] = useAppPreference<string | null>(LAST_BASKET_PREF, null);
  const field = useRef<HTMLTextAreaElement | null>(null);
  const nameField = useRef<HTMLInputElement | null>(null);
  const title = useContextTitle(add.what);
  const resolved = useResolvedTitle(add.what.label ? null : add.what.uri);
  const names = baskets.named.map((b) => b.name);
  // The person's pick, else where to start: the basket last added to.
  const start = choiceOf(defaultDestination(add.thread, names, last));
  const choice = add.choice ?? start;

  // Focus the note once the panel is up (the panel takes focus as it opens).
  useEffect(() => {
    const t = setTimeout(() => field.current?.focus(), 0);
    return () => clearTimeout(t);
  }, [add.what]);

  useEffect(() => {
    if (choice === NEW_MARK) nameField.current?.focus();
  }, [choice]);

  const to = destinationOf(add, choice);
  const submit = () => {
    if (!to) return;
    setLast(rememberedAs(to));
    onAdded?.(to);
    void submitToContext(add, to, resolved);
  };
  const keys = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancelAddToContext();
    }
  };
  const Icon = add.what.text ? Quote : FileText;
  return (
    <div data-testid="context-add-card" role="group" aria-label="Add to Context" onKeyDown={keys} style={cardStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
        <span style={{ color: "var(--text-muted)" }}>Adding</span>
        <Icon size={14} aria-hidden style={{ flexShrink: 0 }} />
        <span data-testid="context-add-title" style={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {title}
        </span>
      </div>
      {add.what.text ? <div style={quoteStyle}>{add.what.text.slice(0, 300)}</div> : null}
      <textarea
        ref={field}
        data-testid="context-note-field"
        aria-label="Note"
        placeholder="Note for the agent (optional): what should it do about this?"
        value={add.note}
        rows={2}
        onChange={(e) => updatePendingAdd({ note: e.target.value })}
        style={{ ...inputStyle, resize: "vertical" }}
      />
      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 8 }}>
        <div role="radiogroup" aria-label="To" style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 6, flex: "1 1 auto", minWidth: 0 }}>
          <span style={{ color: "var(--text-muted)", marginRight: 2 }}>To</span>
          <Chip value={THREAD_MARK} choice={choice} disabledReason={add.thread ? null : "Select a thread first"}>
            {THREAD_BASKET}
          </Chip>
          {names.map((n) => (
            <Chip key={n} value={n} choice={choice}>
              {n}
            </Chip>
          ))}
          {choice === NEW_MARK ? (
            <input
              ref={nameField}
              data-testid="context-note-new-basket"
              aria-label="New basket's name"
              placeholder="New basket's name"
              value={add.newName}
              maxLength={60}
              onChange={(e) => updatePendingAdd({ newName: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
                  e.preventDefault();
                  submit();
                } else if (e.key === "Escape" && names.length + (add.thread ? 1 : 0) > 0) {
                  // Back to the choices; with nothing else to pick, Escape drops the add.
                  e.preventDefault();
                  e.stopPropagation();
                  updatePendingAdd({ choice: start === NEW_MARK ? THREAD_MARK : start });
                }
              }}
              style={{ ...inputStyle, flex: "1 1 140px", padding: "3px 8px" }}
            />
          ) : (
            <Chip value={NEW_MARK} choice={choice}>
              + New basket
            </Chip>
          )}
        </div>
        <span style={{ color: "var(--text-muted)", marginLeft: "auto" }}>{formatShortcut("Ctrl/Cmd+Enter")} adds</span>
        <button type="button" className="ox-button" data-testid="context-note-cancel" onClick={() => cancelAddToContext()}>
          Cancel
        </button>
        <button type="button" className="ox-button ox-primary" data-testid="context-note-add" disabled={!to} title={to ? undefined : "Name the new basket"} onClick={submit}>
          Add
        </button>
      </div>
    </div>
  );
}

function Chip({ value, choice, disabledReason = null, children }: { value: string; choice: string; disabledReason?: string | null; children: ReactNode }) {
  const on = value === choice;
  return (
    <button
      type="button"
      role="radio"
      aria-checked={on}
      data-testid="context-add-to"
      data-value={value}
      disabled={disabledReason != null}
      title={disabledReason ?? undefined}
      onClick={() => updatePendingAdd({ choice: value })}
      style={{
        border: `1px solid ${on ? "var(--accent)" : "var(--border-subtle)"}`,
        background: on ? "var(--accent-soft-bg)" : "transparent",
        color: disabledReason ? "var(--text-muted)" : "var(--text-primary)",
        borderRadius: 999,
        padding: "2px 10px",
        font: "inherit",
        cursor: disabledReason ? "default" : "pointer",
        opacity: disabledReason ? 0.6 : 1,
      }}
    >
      {children}
    </button>
  );
}

const cardStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 8,
  padding: 10,
  marginBottom: 6,
  // The panel keeps its full width; the card holds a note's worth.
  maxWidth: 680,
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  fontSize: "var(--text-xs)",
};

const quoteStyle: CSSProperties = {
  color: "var(--text-secondary)",
  borderLeft: "2px solid var(--border-subtle)",
  paddingLeft: 8,
  whiteSpace: "pre-wrap",
  maxHeight: 60,
  overflow: "hidden",
  fontFamily: "var(--font-mono, monospace)",
};

const inputStyle: CSSProperties = {
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  background: "var(--surface-elevated)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "6px 8px",
};
