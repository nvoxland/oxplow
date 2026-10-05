/// The effort page's first line (tsk1036): did it test, how much of the
/// change ran, and what's left to check — before the sections that explain.
import { useCallback, useEffect, useState } from "react";

import { querySql } from "../api.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { VERDICT_SQL, verdictItems, verdictOf, type VerdictItem } from "./effortVerdict.js";

const TONE: Record<VerdictItem["tone"], string> = {
  good: "var(--status-done)",
  bad: "var(--status-waiting)",
  neutral: "var(--text-secondary)",
};

export function EffortVerdict({ effortRow }: { effortRow: number }) {
  const [items, setItems] = useState<VerdictItem[] | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(async () => {
    try {
      const result = await querySql(VERDICT_SQL, [effortRow]);
      setItems(verdictItems(verdictOf(result)));
      setReads(result.reads);
    } catch (e) {
      recordOpError({ label: "Read the effort's verdict", message: e instanceof Error ? e.message : String(e) });
    }
  }, [effortRow]);
  useEffect(() => {
    void load();
  }, [load]);
  useRerunOnChange(reads, () => void load());
  if (!items) return null;
  return (
    <div
      data-testid="effort-verdict"
      style={{
        display: "flex",
        flexWrap: "wrap",
        gap: 8,
        padding: "8px 10px",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        background: "var(--surface-card)",
        fontSize: "var(--text-sm)",
      }}
    >
      {items.map((item) => (
        <span key={item.key} data-testid={`effort-verdict-${item.key}`} data-tone={item.tone} style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
          <span aria-hidden style={{ width: 8, height: 8, borderRadius: 4, background: TONE[item.tone] }} />
          {item.text}
        </span>
      ))}
    </div>
  );
}
