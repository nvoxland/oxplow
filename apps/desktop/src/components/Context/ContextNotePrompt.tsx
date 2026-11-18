/// The prompt every Add to Context opens: what is being added, an optional
/// note of what the person wants the agent to do about it, and **To**, the
/// basket it goes in — This Thread (the thread on screen's own), a named
/// basket, or New basket…, which asks for a name. One for the app (`App`
/// mounts it); `addToContext` opens it. It starts at the basket last added
/// to (an app preference). It sits where the person last pressed (a menu item), kept on screen.
/// Ctrl/Cmd+Enter or Add adds — with no note too; Enter is a new line in the
/// note, and adds from the new basket's name; Escape, Cancel, or a click
/// outside before anything is typed adds nothing (`.context/usability.md`
/// "Enter submits").

import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { formatShortcut } from "../../keybindings.js";
import { useAppPreference } from "../../prefs/remembered.js";
import {
  NO_BASKETS,
  THREAD_BASKET,
  cancelAddToContext,
  LAST_BASKET_PREF,
  THREAD_MARK,
  defaultDestination,
  rememberedAs,
  submitToContext,
  useBaskets,
  usePendingAdd,
  type ContextWhat,
  type Destination,
} from "../../contextBasket.js";

const WIDTH = 380;
const HEIGHT = 230;
const THREAD = THREAD_MARK;
const NEW = "@new";

// Where the person last pressed: the prompt opens there.
let lastPress: { x: number; y: number } | null = null;
if (typeof document !== "undefined") {
  const note = (e: MouseEvent) => {
    lastPress = { x: e.clientX, y: e.clientY };
  };
  document.addEventListener("mousedown", note, true);
  document.addEventListener("contextmenu", note, true);
}

/** What the prompt calls the item: its label, else its page, else its text's start. */
export function whatTitle(what: ContextWhat): string {
  return what.label || what.uri || (what.text ?? "").slice(0, 80);
}

/** The picker's value for `to`. */
function choiceOf(to: Destination | null): string {
  if (!to) return NEW;
  return "thread" in to ? THREAD : to.basket;
}

export function ContextNotePrompt() {
  const add = usePendingAdd();
  const baskets = useBaskets(add?.thread ?? null);
  const [last, setLast] = useAppPreference<string | null>(LAST_BASKET_PREF, null);
  const names = baskets.named.map((b) => b.name);
  const [note, setNote] = useState("");
  const [choice, setChoice] = useState(THREAD);
  const [newName, setNewName] = useState("");
  const [at, setAt] = useState<{ left: number; top: number }>({ left: 0, top: 0 });
  const box = useRef<HTMLDivElement | null>(null);
  const field = useRef<HTMLTextAreaElement | null>(null);
  const nameField = useRef<HTMLInputElement | null>(null);
  // The add the picker was set for: it starts at its default once per add,
  // when the baskets are read, and is the person's after that.
  const defaulted = useRef<typeof add>(null);

  useEffect(() => {
    if (!add) return;
    setNote("");
    setNewName("");
    const x = lastPress?.x ?? window.innerWidth / 2 - WIDTH / 2;
    const y = lastPress?.y ?? 80;
    setAt({
      left: Math.min(Math.max(8, x), window.innerWidth - WIDTH - 8),
      top: Math.min(Math.max(8, y + 8), window.innerHeight - HEIGHT - 8),
    });
    field.current?.focus();
  }, [add]);

  // The basket to start at, once the baskets are read.
  useEffect(() => {
    if (!add || defaulted.current === add || baskets === NO_BASKETS) return;
    defaulted.current = add;
    setChoice(choiceOf(defaultDestination(add.thread, baskets.named.map((b) => b.name), last)));
  }, [add, baskets, last]);

  useEffect(() => {
    if (choice === NEW) nameField.current?.focus();
  }, [choice]);

  useEffect(() => {
    if (!add) return;
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node) && !field.current?.value.trim()) cancelAddToContext();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [add]);

  if (!add) return null;
  const to: Destination | null =
    choice === THREAD ? (add.thread ? { thread: add.thread } : null) : choice === NEW ? (newName.trim() ? { basket: newName.trim() } : null) : { basket: choice };
  const submit = () => {
    if (!to) return;
    setLast(rememberedAs(to));
    void submitToContext(add, to, note);
  };
  const keys = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelAddToContext();
    }
  };
  return (
    <div
      ref={box}
      data-testid="context-note-prompt"
      role="dialog"
      aria-label="Add to Context"
      onKeyDown={keys}
      // Keep the press inside from reaching a host that would take focus back.
      onMouseDown={(e) => e.stopPropagation()}
      style={{
        position: "fixed",
        left: at.left,
        top: at.top,
        zIndex: 1000,
        width: WIDTH,
        display: "flex",
        flexDirection: "column",
        gap: 8,
        padding: 12,
        background: "var(--surface-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: 8,
        boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--text-sm)",
      }}
    >
      <div style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        Add to Context: <span style={{ color: "var(--text-primary)" }}>{whatTitle(add.what)}</span>
      </div>
      <textarea
        ref={field}
        data-testid="context-note-field"
        aria-label="Note"
        placeholder="Note for the agent (optional): what should it do about this?"
        value={note}
        rows={3}
        onChange={(e) => setNote(e.target.value)}
        style={{ ...fieldStyle, resize: "vertical" }}
      />
      <label style={{ display: "flex", alignItems: "center", gap: 8, color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>
        To
        {choice === NEW ? (
          <input
            ref={nameField}
            data-testid="context-note-new-basket"
            aria-label="New basket's name"
            placeholder="New basket's name"
            value={newName}
            maxLength={60}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
                e.preventDefault();
                submit();
              } else if (e.key === "Escape" && names.length + (add.thread ? 1 : 0) > 0) {
                // Back to the picker; with nothing else to pick, Escape closes the prompt.
                e.preventDefault();
                e.stopPropagation();
                setChoice(choiceOf(defaultDestination(add.thread, names, last)));
              }
            }}
            style={{ ...fieldStyle, flex: 1 }}
          />
        ) : (
          <select data-testid="context-note-basket" aria-label="Basket" value={choice} onChange={(e) => setChoice(e.target.value)} style={{ ...fieldStyle, flex: 1 }}>
            <option value={THREAD} disabled={!add.thread} title={add.thread ? undefined : "Select a thread first"}>
              {add.thread ? THREAD_BASKET : `${THREAD_BASKET} (select a thread first)`}
            </option>
            {names.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
            <option value={NEW}>New basket…</option>
          </select>
        )}
      </label>
      <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 8 }}>
        <span style={{ flex: 1, color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>{formatShortcut("Ctrl/Cmd+Enter")} adds</span>
        <button type="button" className="ox-button" data-testid="context-note-cancel" onClick={() => cancelAddToContext()}>
          Cancel
        </button>
        <button type="button" className="ox-button ox-primary" data-testid="context-note-add" disabled={!to} onClick={submit}>
          Add
        </button>
      </div>
    </div>
  );
}

const fieldStyle = {
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "6px 8px",
} as const;
