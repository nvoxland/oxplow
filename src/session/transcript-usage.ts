import { readFileSync } from "node:fs";
import type { Logger } from "../core/logger.js";
import type { TurnUsage } from "../persistence/turn-store.js";

/**
 * Claude Code writes one JSON object per line to the session transcript at
 * `~/.claude/projects/<slug>/<sessionId>.jsonl`. Assistant entries carry a
 * `message.usage` object with token counts per request iteration. Given the
 * transcript text and a turn's ISO start timestamp, return the summed usage
 * across every assistant entry whose `timestamp` falls on or after that
 * boundary — i.e. the tokens spent servicing that single user prompt.
 *
 * A turn with no matching assistant entries (e.g. the agent got the prompt
 * but errored before any LLM call) returns a record of all-nulls; callers
 * use that to skip the DB update rather than persist a misleading zero.
 */
export function summarizeTurnUsage(
  transcriptText: string,
  turnStartedAtIso: string,
): TurnUsage {
  const turnStartMs = Date.parse(turnStartedAtIso);
  let input = 0;
  let output = 0;
  let cacheRead = 0;
  let matched = 0;

  for (const rawLine of transcriptText.split("\n")) {
    const line = rawLine.trim();
    if (!line) continue;
    let entry: unknown;
    try { entry = JSON.parse(line); } catch { continue; }
    if (!entry || typeof entry !== "object") continue;
    const obj = entry as Record<string, unknown>;
    if (obj.type !== "assistant") continue;
    const ts = typeof obj.timestamp === "string" ? Date.parse(obj.timestamp) : Number.NaN;
    if (!Number.isFinite(turnStartMs) || !Number.isFinite(ts) || ts < turnStartMs) continue;
    const message = obj.message;
    if (!message || typeof message !== "object") continue;
    const usage = (message as Record<string, unknown>).usage;
    if (!usage || typeof usage !== "object") continue;
    const u = usage as Record<string, unknown>;
    input += asNumber(u.input_tokens);
    output += asNumber(u.output_tokens);
    cacheRead += asNumber(u.cache_read_input_tokens);
    matched += 1;
  }

  if (matched === 0) {
    return { inputTokens: null, outputTokens: null, cacheReadInputTokens: null };
  }
  return { inputTokens: input, outputTokens: output, cacheReadInputTokens: cacheRead };
}

function asNumber(raw: unknown): number {
  if (typeof raw !== "number" || !Number.isFinite(raw)) return 0;
  return raw;
}

/**
 * Read a transcript file safely (returns null on any IO error / missing file)
 * and delegate to `summarizeTurnUsage`. Runtime glue that keeps the pure
 * function above testable without touching the filesystem.
 */
export function readTurnUsage(
  transcriptPath: string,
  turnStartedAtIso: string,
  logger?: Logger,
): TurnUsage | null {
  try {
    const text = readFileSync(transcriptPath, "utf8");
    return summarizeTurnUsage(text, turnStartedAtIso);
  } catch (error) {
    logger?.warn("failed to read transcript for token usage", {
      transcriptPath,
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}
