import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";

export interface TabsProps {
  /** What the tabs choose between, for screen readers. */
  label: string;
  tabs: { id: string; title: string }[];
  /** The tab in view. */
  current: string | null;
  /** Picks a tab; absent, the tabs only say which is in view. */
  onPick?(id: string): void;
  testId?: string;
  /** The panel of the tab in view. */
  children: ReactNode;
}

/** A row of tabs over the one panel in view: a page's `switch` inside its
 *  body. Left and Right move to the previous and next tab, and wrap. */
export function Tabs({ label, tabs, current, onPick, testId, children }: TabsProps) {
  const base = useId();
  const strip = useRef<HTMLDivElement>(null);
  const tabId = (id: string) => `${base}-tab-${id}`;
  const pick = (id: string) => {
    if (id !== current) onPick?.(id);
  };
  const onKeyDown = (e: KeyboardEvent, index: number) => {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (step === 0 || tabs.length === 0) return;
    e.preventDefault();
    const next = tabs[(index + step + tabs.length) % tabs.length]!;
    pick(next.id);
    strip.current?.querySelector<HTMLElement>(`[data-tab="${next.id}"]`)?.focus();
  };
  return (
    <div className="ox-tabs" data-testid={testId}>
      <div role="tablist" aria-label={label} className="ox-tabs-strip" ref={strip}>
        {tabs.map((t, i) => {
          const on = t.id === current;
          return (
            <button
              key={t.id}
              id={tabId(t.id)}
              type="button"
              role="tab"
              className="ox-tab"
              data-tab={t.id}
              data-testid={testId ? `${testId}-tab-${t.id}` : undefined}
              aria-selected={on}
              tabIndex={on ? 0 : -1}
              disabled={!onPick && !on}
              onClick={() => pick(t.id)}
              onKeyDown={(e) => onKeyDown(e, i)}
            >
              {t.title}
            </button>
          );
        })}
      </div>
      <div role="tabpanel" className="ox-tabs-panel" aria-labelledby={current ? tabId(current) : undefined}>
        {children}
      </div>
    </div>
  );
}
