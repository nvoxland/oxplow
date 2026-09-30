/// A thread's answers, kept live: re-read when `v_thread_answer` changes
/// (a new `show_lens`, a Keep This). The Answers strip and an ACP
/// transcript share it.
import { useCallback, useEffect, useState } from "react";

import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import type { Reads } from "../../tauri-bridge/generated/bindings.js";
import { readAnswers, type AnswerRow } from "../../threadAnswers.js";

export function useThreadAnswers(threadId: string): AnswerRow[] {
  const [answers, setAnswers] = useState<AnswerRow[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const refresh = useCallback(async () => {
    try {
      const out = await readAnswers(threadId);
      setAnswers(out.answers);
      setReads(out.reads);
    } catch {
      // The next change re-reads.
    }
  }, [threadId]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  return answers;
}
