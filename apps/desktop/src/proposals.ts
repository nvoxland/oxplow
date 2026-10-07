/**
 * An agent's runs that wait for a person (P6b, `.context/commands.md` →
 * "Proposals"): read from `v_command_proposal`, decided through RPC
 * `decide_proposal`. Approving runs the command as the person — the
 * approval is the confirmation; declining runs nothing.
 */
import { useCallback, useEffect, useState } from "react";

import { decideProposal, querySql, type AcpToolCall } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import { threadRowId } from "./modelIds.js";
import { valueText } from "./pages/settingsModel.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** What the run raised: the command, its summary and input, and whether
 *  it is destructive. */
export interface ProposalPreview {
  command: string;
  summary: string;
  input: unknown;
  destructive: boolean;
}

/** One pending proposal. */
export interface Proposal {
  id: number;
  /** `proposal:<id>`. */
  ref: string;
  createdAt: string;
  command: string;
  input: unknown;
  /** `agent`, or `lens` for a lens acting for an agent. */
  actorKind: string;
  /** The proposing thread (`thr3`) or lens. */
  actorId: string | null;
  threadId: number | null;
  /** The proposing thread's title, as a person knows it (tsk1044). */
  threadTitle: string | null;
  /** `config:<key>` for a config change, else the command and its input. */
  key: string;
  preview: ProposalPreview;
  /** What it would have done when proposed; `null` when it couldn't be
   *  dry-run (an External command). */
  dryRun: unknown;
  /** `pending`, `approved`, `declined` or `superseded`. */
  decision: string;
  /** When it was decided or superseded. */
  decidedAt: string | null;
  /** The approving run's audit row: none while an approved one still
   *  runs (an External command is claimed `approved` first, and goes back
   *  to `pending` if the run fails). */
  auditId: number | null;
}

/** How a card shows a proposal. */
export interface ProposalSummary {
  title: string;
  who: string;
  /** A config key's value before and after. */
  change: { before: string; after: string } | null;
  /** A composite's commands, in order. */
  children: string[];
  destructive: boolean;
}

const json = (text: SqlCell): unknown => {
  if (text == null) return null;
  try {
    return JSON.parse(String(text));
  } catch {
    return null;
  }
};

export function proposalsFromResult(result: SqlQueryResult): Proposal[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: Number(at(row, "id")),
    ref: String(at(row, "ref")),
    createdAt: String(at(row, "created_at") ?? ""),
    command: String(at(row, "command")),
    input: json(at(row, "input")),
    actorKind: String(at(row, "actor_kind")),
    actorId: at(row, "actor_id") == null ? null : String(at(row, "actor_id")),
    threadId: at(row, "thread_id") == null ? null : Number(at(row, "thread_id")),
    threadTitle: at(row, "thread_title") == null ? null : String(at(row, "thread_title")),
    key: String(at(row, "key")),
    preview: json(at(row, "preview")) as ProposalPreview,
    dryRun: json(at(row, "dry_run")),
    // A read of the pending ones needn't select it.
    decision: String(at(row, "decision") ?? "pending"),
    decidedAt: at(row, "decided_at") == null ? null : String(at(row, "decided_at")),
    auditId: at(row, "audit_id") == null ? null : Number(at(row, "audit_id")),
  }));
}

/** The pending proposals, newest first, and what was read. */
export async function readPendingProposals(): Promise<{ proposals: Proposal[]; reads: Reads }> {
  const res = await querySql(
    `SELECT id, ref, created_at, command, input, actor_kind, actor_id, thread_id, ${THREAD_TITLE}, key, preview, dry_run
       FROM v_command_proposal p WHERE decision = 'pending' ORDER BY id DESC`,
    [],
    1_000,
  );
  return { proposals: proposalsFromResult(res), reads: res.reads };
}

/** The pending proposals, re-read when the model changes. */
export function useProposals(): Proposal[] {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readPendingProposals()
      .then((r) => {
        setProposals(r.proposals);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        // The rail and Settings keep what they showed; the next change
        // retries. The person learns the list may be stale.
        recordOpError({ label: "Read pending proposals", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return proposals;
}

/** The proposing thread's title, by the proposal's `thread_id`. */
const THREAD_TITLE = "(SELECT title FROM v_thread th WHERE th.id = p.thread_id) AS thread_title";

const field = (value: unknown, name: string): unknown =>
  value && typeof value === "object" ? (value as Record<string, unknown>)[name] : undefined;

export function summarizeProposal(p: Proposal): ProposalSummary {
  const key = field(p.input, "key");
  const title =
    p.command === "oxplow.config.set" && typeof key === "string"
      ? `Set ${key}`
      : p.command === "oxplow.config.unset" && typeof key === "string"
        ? `Unset ${key}`
        : p.preview?.summary || p.command;
  // The thread by its title, as the person knows it, never its id.
  const agent = p.threadTitle ? `The agent in “${p.threadTitle}”` : "The agent";
  const who = p.actorKind === "lens" ? `A lens${p.actorId ? ` (${p.actorId})` : ""} for the agent` : agent;
  const hasChange = p.dryRun != null && typeof p.dryRun === "object" && "before" in p.dryRun && "after" in p.dryRun;
  const shown = (v: unknown) => (v == null ? "(not set)" : valueText(v));
  const children = field(p.dryRun, "children");
  return {
    title,
    who,
    change: hasChange ? { before: shown(field(p.dryRun, "before")), after: shown(field(p.dryRun, "after")) } : null,
    children: Array.isArray(children) ? children.map((c) => String(field(c, "name"))) : [],
    destructive: Boolean(p.preview?.destructive),
  };
}

/** The pending proposal that changes setting `key`, if any. */
export function proposalForSetting(proposals: Proposal[], key: string): Proposal | undefined {
  return proposals.find((p) => p.key === `config:${key}`);
}

/** Approve or decline `p` as the person. */
export async function decide(p: Proposal, approve: boolean): Promise<void> {
  await decideProposal(p.id, approve);
}

const COLUMNS =
  "id, ref, created_at, command, input, actor_kind, actor_id, thread_id, (SELECT title FROM v_thread th WHERE th.id = p.thread_id) AS thread_title, key, preview, dry_run, decision, decided_at, audit_id";

/** A thread's proposals, newest first — pending and decided — and what
 *  was read. */
export async function readThreadProposals(threadId: string): Promise<{ proposals: Proposal[]; reads: Reads }> {
  const res = await querySql(
    `SELECT ${COLUMNS} FROM v_command_proposal p WHERE thread_id = ?1 ORDER BY id DESC`,
    [threadRowId(threadId)],
    1_000,
  );
  return { proposals: proposalsFromResult(res), reads: res.reads };
}

/** A thread's proposals, re-read when the model changes: what waits for
 *  the person where the conversation is (the Answers strip, an ACP
 *  transcript), and what became of the ones they decided. */
export function useThreadProposals(threadId: string): Proposal[] {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readThreadProposals(threadId)
      .then((r) => {
        setProposals(r.proposals);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read this thread's proposals", message: e instanceof Error ? e.message : String(e) });
      });
  }, [threadId]);
  useEffect(() => {
    setProposals([]);
    load();
  }, [load]);
  useRerunOnChange(reads, load);
  return proposals;
}

/** The proposal a tool call left (`proposal:N`), so an ACP transcript shows
 *  it where the agent asked. `run_command` answers `{ kind: "proposed",
 *  proposal }`; a lens action's refusal says "recorded as proposal:N".
 *  Only oxplow's command tools: another tool quoting such text names none. */
export function proposalOfTool(call: AcpToolCall): string | null {
  const names = [call.name, call.title].filter((n): n is string => n !== null);
  if (!names.some((n) => /^mcp__\S*__(run_command|run_lens_action)\b/.test(n))) return null;
  const output = [...call.text, call.rawOutput === null ? "" : JSON.stringify(call.rawOutput)].join("\n");
  const m = /\\*"proposal\\*"\s*:\s*\\*"(proposal:[0-9]+)\\*"|recorded as (proposal:[0-9]+)/.exec(output);
  return m?.[1] ?? m?.[2] ?? null;
}
